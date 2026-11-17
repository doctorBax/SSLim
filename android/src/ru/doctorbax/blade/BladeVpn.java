package ru.doctorbax.blade;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.net.VpnService;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructPollfd;
import android.util.Log;

import java.io.BufferedInputStream;
import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.FileDescriptor;
import java.io.IOException;
import java.io.InputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.security.cert.CertificateFactory;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManagerFactory;

/**
 * TUN service. Every connection parameter comes from the active profile via
 * intent extras, so the same public build works with any SSLim server.
 */
public final class BladeVpn extends VpnService {
    static volatile boolean running;
    static volatile String status = "Отключено";
    static volatile String activeName = "";
    static final String DISCONNECT = "ru.doctorbax.blade.DISCONNECT";

    private static final int[] QUICK_DELAYS_MS = {250, 750, 1500};
    private static final int RECONNECT_MAX_MS = 30_000;
    private static final int MTU = 1280;
    private static final int TUN_POLL_MS = 250;
    private static final int TUN_QUEUE_CAPACITY = 64;
    private static final int MAX_BATCH_PACKETS = 32;
    private static final int MAX_BATCH_BYTES = 32 * 1024;
    private static final long PERF_REPORT_MS = 10_000;
    private static final byte[] PING_FRAME = {2, 0, 0};
    private static final byte[] PONG_FRAME = {3, 0, 0};
    private static final byte[] CLOSE_FRAME = {4, 0, 0};

    static final class Traffic {
        final AtomicLong upload = new AtomicLong();
        final AtomicLong download = new AtomicLong();
        final AtomicLong pingStarted = new AtomicLong();
        volatile long rttMs = -1;
        volatile long rttSampleTime;
        final AtomicLong uplinkPackets = new AtomicLong();
        final AtomicLong uplinkBatches = new AtomicLong();
        final AtomicLong tlsSlowWrites = new AtomicLong();
        final AtomicLong tunBackpressure = new AtomicLong();
        final AtomicLong tunWaitMs = new AtomicLong();
        final AtomicLong tunQueuePeak = new AtomicLong();
    }

    /** Fixed-size reusable buffers: bound memory even if Android TUN stalls. */
    private static final class PacketSlot {
        final byte[] data = new byte[MTU];
        int size;
    }

    static volatile Traffic traffic = new Traffic();
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable forceClose = this::closeResources;
    private volatile boolean stopping;
    private Thread worker;
    private Thread closeWorker;
    private DataOutputStream sessionOutput;
    private String phase;
    private Socket socket;
    private ParcelFileDescriptor tun;
    private volatile ProfileStore.Profile config;

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && DISCONNECT.equals(intent.getAction())) {
            disconnect();
            return START_NOT_STICKY;
        }
        if (running) return START_NOT_STICKY;
        config = readConfig(intent);
        if (config == null) {
            status = "Нет профиля: откройте Blade и заполните настройки";
            return START_NOT_STICKY;
        }
        running = true;
        status = "Подключение…";
        activeName = config.name;
        startForeground(1, notification());
        stopping = false;
        worker = new Thread(this::runTunnel, "blade-vpn");
        worker.start();
        return START_NOT_STICKY;
    }

    private ProfileStore.Profile readConfig(Intent intent) {
        if (intent == null) return null;
        String host = intent.getStringExtra("host");
        String secret = intent.getStringExtra("secretBase64");
        if (host == null || host.isEmpty() || secret == null) return null;
        try {
            byte[] key = java.util.Base64.getMimeDecoder().decode(secret);
            if (key.length < 32) return null;
            return new ProfileStore.Profile("active",
                intent.getStringExtra("name") == null ? host : intent.getStringExtra("name"),
                host,
                intent.getIntExtra("port", 9443),
                intent.getStringExtra("sni") == null ? host : intent.getStringExtra("sni"),
                intent.getStringExtra("marker") == null ? "sync/1" : intent.getStringExtra("marker"),
                key,
                intent.getStringExtra("caPem"));
        } catch (IllegalArgumentException error) {
            return null;
        }
    }

    private SSLSocketFactory serverSocketFactory() throws Exception {
        SSLContext context = SSLContext.getInstance("TLS");
        if (config.caPem == null || config.caPem.isEmpty()) {
            context.init(null, null, null);
            return context.getSocketFactory();
        }
        KeyStore anchors = KeyStore.getInstance(KeyStore.getDefaultType());
        anchors.load(null, null);
        try (InputStream pem = new ByteArrayInputStream(config.caPem.getBytes(StandardCharsets.US_ASCII))) {
            anchors.setCertificateEntry("blade-ca",
                CertificateFactory.getInstance("X.509").generateCertificate(pem));
        }
        TrustManagerFactory trust = TrustManagerFactory.getInstance(
            TrustManagerFactory.getDefaultAlgorithm());
        trust.init(anchors);
        context.init(null, trust.getTrustManagers(), null);
        return context.getSocketFactory();
    }

    private Notification notification() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(
            new NotificationChannel("vpn", "VPN", NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0,
            new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, "vpn")
            .setSmallIcon(R.drawable.ic_arrow_right)
            .setContentTitle("Blade")
            .setContentText("Нажмите, чтобы открыть управление VPN")
            .setContentIntent(open).setOngoing(true).build();
    }

    private void runTunnel() {
        long delay = 0;
        int attempt = 0;
        try {
            while (!stopping) {
                try {
                    runSession();
                    return; // server closed cleanly or stopping was requested
                } catch (Exception error) {
                    if (stopping) return;
                    Log.w("Blade", "VPN failed during " + phase, error);
                    delay = nextDelay(attempt++);
                    status = "Переподключение через " + (delay / 1000) + " с…";
                    try {
                        Thread.sleep(delay);
                    } catch (InterruptedException interrupt) {
                        return;
                    }
                }
            }
        } finally {
            closeResources();
            Thread closing = closeWorker;
            if (closing != null && closing != Thread.currentThread()) {
                boolean interrupted = Thread.interrupted();
                try {
                    closing.join(600);
                } catch (InterruptedException error) {
                    interrupted = true;
                }
                if (interrupted) Thread.currentThread().interrupt();
            }
            handler.post(this::finishTunnel);
        }
    }

    private void finishTunnel() {
        handler.removeCallbacks(forceClose);
        synchronized (this) {
            running = false;
            worker = null;
            closeWorker = null;
            if (stopping || "Подключение…".equals(status) || "VPN подключён".equals(status)) {
                status = "Отключено";
            }
            stopping = false;
        }
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    private long nextDelay(int attempt) {
        if (attempt < QUICK_DELAYS_MS.length) return QUICK_DELAYS_MS[attempt];
        long delay = 1000;
        for (int i = QUICK_DELAYS_MS.length; i < attempt; i++) {
            delay = Math.min(delay * 2, RECONNECT_MAX_MS);
        }
        return delay;
    }

    private void runSession() throws Exception {
        ProfileStore.Profile profile = config;
        phase = "соединение с сервером";
        try (Socket raw = new Socket()) {
            synchronized (this) {
                if (stopping) return;
                socket = raw;
            }
            raw.bind(new InetSocketAddress(0));
            if (!protect(raw)) throw new IOException("VPN permission revoked");
            InetSocketAddress endpoint = new InetSocketAddress(profile.host, profile.port);
            raw.connect(endpoint, 10000);
            raw.setTcpNoDelay(true);
            raw.setSoTimeout(10000);
            SSLSocketFactory factory = serverSocketFactory();
            try (SSLSocket tls = (SSLSocket) factory.createSocket(raw, profile.host, profile.port, true)) {
                phase = "проверка TLS";
                tls.setEnabledProtocols(new String[]{"TLSv1.3"});
                SSLParameters parameters = tls.getSSLParameters();
                parameters.setEndpointIdentificationAlgorithm("HTTPS");
                parameters.setApplicationProtocols(new String[]{"http/1.1"});
                // SNI only for real hostnames; Java rejects IP literals here,
                // and an IP profile legitimately connects without SNI.
                if (profile.serverName.matches("[0-9.]+")) {
                    parameters.setServerNames(java.util.Collections.emptyList());
                } else {
                    parameters.setServerNames(java.util.Collections.singletonList(
                        new javax.net.ssl.SNIHostName(profile.serverName)));
                }
                tls.setSSLParameters(parameters);
                tls.startHandshake();
                phase = "авторизация";
                DataInputStream input = new DataInputStream(new BufferedInputStream(tls.getInputStream(), 32 * 1024));
                DataOutputStream output = new DataOutputStream(tls.getOutputStream());
                final String address;
                try {
                    address = authenticate(input, output, profile);
                } catch (EOFException error) {
                    throw new IOException("Сервер отклонил соединение", error);
                }
                tls.setSoTimeout(0);
                phase = "запуск VPN";
                final ParcelFileDescriptor device;
                Traffic counters = new Traffic();
                synchronized (this) {
                    if (stopping) return;
                    device = new Builder().setSession("Blade")
                        .setMtu(MTU).addAddress(address, 32)
                        .addRoute("0.0.0.0", 0)
                        .addDnsServer("1.1.1.1").addDnsServer("9.9.9.9")
                        .setBlocking(false).establish();
                    if (device == null) throw new IOException("VPN permission revoked");
                    tun = device;
                    sessionOutput = output;
                    traffic = counters;
                    status = "VPN подключён";
                }
                phase = "передача данных";
                // Decouple network reads from potentially blocking Android TUN writes.
                // The two bounded queues also provide explicit backpressure with no
                // per-packet allocations on the downlink hot path.
                final ArrayBlockingQueue<PacketSlot> empty = new ArrayBlockingQueue<>(TUN_QUEUE_CAPACITY);
                final ArrayBlockingQueue<PacketSlot> pending = new ArrayBlockingQueue<>(TUN_QUEUE_CAPACITY);
                for (int i = 0; i < TUN_QUEUE_CAPACITY; i++) empty.add(new PacketSlot());
                Thread sender = new Thread(() -> sendPackets(device, output, counters), "blade-vpn-uplink");
                Thread receiver = new Thread(() -> receivePackets(device, pending, empty, counters),
                    "blade-vpn-tun-writer");
                receiver.start();
                sender.start();
                try {
                    while (!stopping) {
                        int type = input.readUnsignedByte();
                        int size = input.readUnsignedShort();
                        if (type == 4 && size == 0) break;
                        if (type == 3 && size == 0) {
                            long started = counters.pingStarted.getAndSet(0);
                            if (started != 0) {
                                long now = SystemClock.elapsedRealtime();
                                counters.rttMs = now - started;
                                counters.rttSampleTime = now;
                            }
                            continue;
                        }
                        if (type == 2 && size == 0) {
                            synchronized (output) {
                                if (stopping) continue;
                                output.write(PONG_FRAME);
                                output.flush();
                            }
                            continue;
                        }
                        if (type != 1 || size < 20 || size > MTU) {
                            throw new IOException("Invalid frame");
                        }
                        PacketSlot slot = null;
                        try {
                            while (!stopping && (slot = empty.poll(TUN_POLL_MS, TimeUnit.MILLISECONDS)) == null) {
                                if (!receiver.isAlive()) throw new IOException("TUN writer exited");
                            }
                            if (stopping) break;
                            input.readFully(slot.data, 0, size);
                            if ((slot.data[0] >>> 4 & 15) != 4) continue;
                            slot.size = size;
                            while (!stopping && !pending.offer(slot, TUN_POLL_MS, TimeUnit.MILLISECONDS)) {
                                if (!receiver.isAlive()) throw new IOException("TUN writer exited");
                            }
                            if (stopping) break;
                            counters.tunQueuePeak.accumulateAndGet(pending.size(), Math::max);
                            slot = null; // ownership transferred to TUN writer
                        } finally {
                            if (slot != null) empty.offer(slot);
                        }
                    }
                } finally {
                    try { raw.close(); } catch (IOException ignored) {}
                    sender.interrupt();
                    receiver.interrupt();
                    sender.join(500);
                    receiver.join(500);
                }
                if (!stopping) status = "Сервер закрыл соединение";
            }
        } finally {
            closeResources();
        }
    }

    private static StructPollfd pollFd(FileDescriptor fd, int event) {
        StructPollfd descriptor = new StructPollfd();
        descriptor.fd = fd;
        descriptor.events = (short) event;
        return descriptor;
    }

    private static void waitFor(StructPollfd descriptor, StructPollfd[] set) throws Exception {
        try {
            Os.poll(set, TUN_POLL_MS);
        } catch (ErrnoException error) {
            if (error.errno != OsConstants.EINTR) throw error;
            return;
        }
        int closed = OsConstants.POLLERR | OsConstants.POLLHUP | OsConstants.POLLNVAL;
        if ((descriptor.revents & closed) != 0) throw new IOException("TUN closed");
    }

    /** Read all immediately available TUN packets into one TLS write. */
    private void sendPackets(ParcelFileDescriptor device, DataOutputStream output, Traffic counters) {
        byte[] batch = new byte[MAX_BATCH_PACKETS * (MTU + 3)];
        StructPollfd readable = pollFd(device.getFileDescriptor(), OsConstants.POLLIN);
        StructPollfd[] waitSet = {readable};
        long nextPing = SystemClock.elapsedRealtime();
        long nextReport = nextPing + PERF_REPORT_MS;
        try {
            while (!stopping && !Thread.currentThread().isInterrupted()) {
                long now = SystemClock.elapsedRealtime();
                if (now >= nextPing && counters.pingStarted.compareAndSet(0, now)) {
                    synchronized (output) {
                        if (stopping) return;
                        output.write(PING_FRAME);
                        output.flush();
                    }
                    nextPing = now + 3000;
                }
                if (now >= nextReport) {
                    Log.i("BladePerf", "up_bytes=" + counters.upload.get()
                        + " down_bytes=" + counters.download.get()
                        + " up_packets=" + counters.uplinkPackets.get()
                        + " up_batches=" + counters.uplinkBatches.get()
                        + " slow_tls_writes=" + counters.tlsSlowWrites.get()
                        + " tun_eagain=" + counters.tunBackpressure.get()
                        + " tun_wait_ms=" + counters.tunWaitMs.get()
                        + " tun_queue_peak=" + counters.tunQueuePeak.get());
                    nextReport = now + PERF_REPORT_MS;
                }

                int bytes = 0;
                int packets = 0;
                int payloadBytes = 0;
                while (packets < MAX_BATCH_PACKETS && bytes < MAX_BATCH_BYTES) {
                    try {
                        int length = Os.read(readable.fd, batch, bytes + 3, MTU);
                        if (length <= 0) throw new IOException("TUN closed");
                        batch[bytes] = 1;
                        batch[bytes + 1] = (byte) (length >>> 8);
                        batch[bytes + 2] = (byte) length;
                        bytes += length + 3;
                        payloadBytes += length;
                        packets++;
                    } catch (ErrnoException error) {
                        if (error.errno == OsConstants.EINTR) continue;
                        if (error.errno != OsConstants.EAGAIN) throw error;
                        if (packets == 0) waitFor(readable, waitSet);
                        break; // flush without waiting for more packets
                    }
                }
                if (packets == 0) continue;
                long started = SystemClock.elapsedRealtimeNanos();
                synchronized (output) {
                    if (stopping) return;
                    output.write(batch, 0, bytes);
                    output.flush();
                }
                if (SystemClock.elapsedRealtimeNanos() - started > 20_000_000L) {
                    counters.tlsSlowWrites.incrementAndGet();
                }
                counters.upload.addAndGet(payloadBytes);
                counters.uplinkPackets.addAndGet(packets);
                counters.uplinkBatches.incrementAndGet();
            }
        } catch (Exception error) {
            if (!stopping) {
                Log.e("Blade", "Uplink failed", error);
                closeResources();
            }
        }
    }

    /** Consume inbound packets independently of the TLS read loop. */
    private void receivePackets(ParcelFileDescriptor device,
                                ArrayBlockingQueue<PacketSlot> pending,
                                ArrayBlockingQueue<PacketSlot> empty,
                                Traffic counters) {
        StructPollfd writable = pollFd(device.getFileDescriptor(), OsConstants.POLLOUT);
        StructPollfd[] waitSet = {writable};
        try {
            while (!stopping && !Thread.currentThread().isInterrupted()) {
                PacketSlot slot = pending.poll(TUN_POLL_MS, TimeUnit.MILLISECONDS);
                if (slot == null) continue;
                try {
                    if (writePacket(writable, waitSet, slot.data, slot.size, counters)) {
                        counters.download.addAndGet(slot.size);
                    }
                } finally {
                    empty.offer(slot);
                }
            }
        } catch (Exception error) {
            if (!stopping) {
                Log.e("Blade", "Downlink TUN writer failed", error);
                closeResources();
            }
        }
    }

    private boolean writePacket(StructPollfd writable, StructPollfd[] waitSet,
                                byte[] packet, int length, Traffic counters) throws Exception {
        while (!stopping && !Thread.currentThread().isInterrupted()) {
            try {
                if (Os.write(writable.fd, packet, 0, length) != length) {
                    throw new IOException("Partial TUN write");
                }
                return true;
            } catch (ErrnoException error) {
                if (error.errno == OsConstants.EINTR) continue;
                if (error.errno != OsConstants.EAGAIN) throw error;
                counters.tunBackpressure.incrementAndGet();
                long start = SystemClock.elapsedRealtime();
                waitFor(writable, waitSet);
                counters.tunWaitMs.addAndGet(SystemClock.elapsedRealtime() - start);
            }
        }
        return false;
    }

    private String authenticate(DataInputStream input, DataOutputStream output,
                              ProfileStore.Profile profile) throws Exception {
        byte[] nonceBytes = new byte[16];
        new SecureRandom().nextBytes(nonceBytes);
        String nonce = hex(nonceBytes);
        String timestamp = Long.toString(System.currentTimeMillis() / 1000);
        Mac mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(profile.secret, "HmacSHA256"));
        String tag = hex(mac.doFinal(("vpn-bot-lab-transport-v1\0" + timestamp + "\0" + nonce)
            .getBytes(StandardCharsets.US_ASCII)));
        String request = "POST /connect HTTP/1.1\r\nHost: " + profile.serverName
            + "\r\nConnection: Upgrade\r\nUpgrade: " + profile.marker + "\r\nX-Session-Time: " + timestamp
            + "\r\nX-Session-Nonce: " + nonce + "\r\nX-Session-Tag: " + tag
            + "\r\nX-Session-Address: auto\r\n\r\n";
        output.write(request.getBytes(StandardCharsets.US_ASCII));
        output.flush();
        StringBuilder response = new StringBuilder();
        int tail = 0;
        for (int i = 0; i < 8192; i++) {
            int value = input.readUnsignedByte();
            response.append((char) value);
            tail = (tail << 8) | value;
            if (tail == 0x0d0a0d0a) {
                if (response.indexOf("HTTP/1.1 101 Switching Protocols\r\n") == 0) {
                    String header = "X-Session-Address:";
                    for (String line : response.toString().split("\r\n")) {
                        if (line.regionMatches(true, 0, header, 0, header.length())) {
                            String address = line.substring(header.length()).trim();
                            if (!address.matches("10\\.253\\.[0-9]{1,3}\\.[0-9]{1,3}")) {
                                throw new IOException("Invalid tunnel address");
                            }
                            return address;
                        }
                    }
                    return "10.253.0.2";
                }
                break;
            }
        }
        throw new IOException("Session rejected");
    }

    private static String hex(byte[] data) {
        String digits = "0123456789abcdef";
        StringBuilder result = new StringBuilder(data.length * 2);
        for (byte value : data) {
            result.append(digits.charAt((value & 255) >>> 4));
            result.append(digits.charAt(value & 15));
        }
        return result.toString();
    }

    private synchronized void closeResources() {
        sessionOutput = null;
        if (socket != null) {
            try { socket.close(); } catch (IOException ignored) {}
            socket = null;
        }
        if (tun != null) {
            try { tun.close(); } catch (IOException ignored) {}
            tun = null;
        }
    }

    @Override public void onRevoke() {
        disconnect();
    }

    private void disconnect() {
        final DataOutputStream output;
        synchronized (this) {
            if (stopping) return;
            stopping = true;
            status = "Отключение…";
            output = sessionOutput;
        }
        if (output == null) {
            closeResources();
            if (worker == null) {
                running = false;
                status = "Отключено";
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
            } else {
                worker.interrupt();
            }
            return;
        }
        // Bound cleanup even if the socket stalls while flushing the CLOSE frame.
        handler.removeCallbacks(forceClose);
        handler.postDelayed(forceClose, 500);
        Thread closing = new Thread(() -> {
            try {
                synchronized (output) {
                    output.write(CLOSE_FRAME);
                    output.flush();
                }
            } catch (IOException ignored) {
            }
        }, "blade-vpn-close");
        synchronized (this) {
            closeWorker = closing;
        }
        closing.start();
    }

    @Override public void onDestroy() {
        handler.removeCallbacksAndMessages(null);
        stopping = true;
        handler.removeCallbacks(forceClose);
        closeResources();
        if (status.equals("VPN подключён") || status.equals("Подключение…") || status.equals("Отключение…")) status = "Отключено";
        running = false;
        worker = null;
        closeWorker = null;
        super.onDestroy();
    }
}
