package ru.doctorbax.blade;

import android.util.Log;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.cert.X509Certificate;
import java.util.Base64;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;

/**
 * Optional native QUIC v3 transport.
 * If no JNI library is packaged, Auto stays on the existing Java/TCP engine.
 * Never falls back from failed QUIC certificate or authentication verification.
 */
final class BladeQuic {
    static final int STOPPED = 0;
    static final int UNAVAILABLE = 1;
    private static final boolean AVAILABLE;

    static {
        boolean available = false;
        try {
            System.loadLibrary("bladequic");
            available = true;
        } catch (UnsatisfiedLinkError error) {
            Log.w("Blade", "QUIC native library not available, TCP-only", error);
        }
        AVAILABLE = available;
    }

    private BladeQuic() {}

    static boolean available() {
        return AVAILABLE;
    }

    static int connect(BladeVpn service, ProfileStore.Profile profile) throws Exception {
        if (!AVAILABLE) throw new IOException("QUIC is not included in this APK");
        byte[] roots = trustedRoots(profile.caPem);
        return runNative(service, profile.host, profile.port, profile.serverName,
            roots, profile.secret);
    }

    static void stop() {
        if (AVAILABLE) stopNative();
    }

    /**
     * Match Android's normal TLS trust behavior. Explicit custom CAs replace
     * system roots, exactly as in the existing Java TLS connector.
     */
    private static byte[] trustedRoots(String caPem) throws Exception {
        if (caPem != null && !caPem.isEmpty()) {
            return caPem.getBytes(StandardCharsets.US_ASCII);
        }
        TrustManagerFactory factory = TrustManagerFactory.getInstance(
            TrustManagerFactory.getDefaultAlgorithm());
        factory.init((KeyStore) null);
        ByteArrayOutputStream pem = new ByteArrayOutputStream();
        int count = 0;
        for (TrustManager manager : factory.getTrustManagers()) {
            if (!(manager instanceof X509TrustManager)) continue;
            for (X509Certificate cert : ((X509TrustManager) manager).getAcceptedIssuers()) {
                String encoded = Base64.getMimeEncoder(64, new byte[]{'\n'})
                    .encodeToString(cert.getEncoded());
                pem.write("-----BEGIN CERTIFICATE-----\n".getBytes(StandardCharsets.US_ASCII));
                pem.write(encoded.getBytes(StandardCharsets.US_ASCII));
                pem.write("\n-----END CERTIFICATE-----\n".getBytes(StandardCharsets.US_ASCII));
                count++;
            }
        }
        if (count == 0) throw new IOException("Android has no usable trusted certificates");
        return pem.toByteArray();
    }

    private static native int runNative(BladeVpn service, String host, int port,
        String serverName, byte[] trustedPem, byte[] secret) throws IOException;

    private static native void stopNative();
}
