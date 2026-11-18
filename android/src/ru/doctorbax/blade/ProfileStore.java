package ru.doctorbax.blade;

import android.content.Context;
import android.util.AtomicFile;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.ByteArrayOutputStream;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;

/**
 * VPN profiles stored as JSON in app-private storage. Nothing about the server
 * is compiled into the application: every field arrives through a profile
 * (manually, from the clipboard, or via an sslim:// link).
 */
public final class ProfileStore {
    /** Immutable snapshot handed to the VPN service. */
    public static final class Profile {
        public final String id;
        public final String name;
        public final String host;
        public final int port;
        public final String serverName;
        public final String marker;
        public final byte[] secret;
        public final String caPem;

        Profile(String id, String name, String host, int port, String serverName,
                String marker, byte[] secret, String caPem) {
            this.id = id;
            this.name = name;
            this.host = host;
            this.port = port;
            this.serverName = serverName;
            this.marker = marker;
            this.secret = secret;
            this.caPem = caPem;
        }

        public String uri() {
            // Export every connection-critical field, especially the historical
            // TCP marker and the server's private CA. Omitting either breaks
            // importing a working profile on a second device.
            StringBuilder link = new StringBuilder("sslim://")
                .append(host).append(":").append(port)
                .append("?sni=").append(serverName)
                .append("&k=").append(Base64.getEncoder().encodeToString(secret))
                .append("&u=").append(marker)
                .append("&n=").append(encodeUriComponent(name));
            if (caPem != null && !caPem.isEmpty()) {
                link.append("&ca=").append(Base64.getEncoder().encodeToString(
                    caPem.getBytes(StandardCharsets.UTF_8)));
            }
            return link.toString();
        }
    }

    /** Encode user-editable labels without changing existing ASCII profile links. */
    private static String encodeUriComponent(String value) {
        final char[] hex = "0123456789ABCDEF".toCharArray();
        StringBuilder result = new StringBuilder();
        for (byte octet : value.getBytes(StandardCharsets.UTF_8)) {
            int b = octet & 255;
            if ((b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z')
                || (b >= '0' && b <= '9') || b == '-' || b == '_' || b == '.' || b == '~') {
                result.append((char) b);
            } else {
                result.append('%').append(hex[b >>> 4]).append(hex[b & 15]);
            }
        }
        return result.toString();
    }

    private static final String FILE = "profiles.json";
    private static final int MIN_SECRET = 32;

    private static List<Profile> profiles;
    private static String activeId;
    private static File file;

    private ProfileStore() {
    }

    private static synchronized File file(Context context) {
        if (file == null) file = new File(context.getFilesDir(), FILE);
        return file;
    }

    private static synchronized void load(Context context) {
        if (profiles != null) return;
        profiles = new ArrayList<>();
        activeId = null;
        try {
            byte[] raw = readAll(file(context));
            if (raw == null) return;
            JSONObject root = new JSONObject(new String(raw, StandardCharsets.UTF_8));
            activeId = root.optString("active", null);
            JSONArray items = root.optJSONArray("profiles");
            for (int i = 0; items != null && i < items.length(); i++) {
                Profile parsed = decode(items.getJSONObject(i));
                if (parsed != null) profiles.add(parsed);
            }
            if (activeId == null && !profiles.isEmpty()) activeId = profiles.get(0).id;
        } catch (Exception ignored) {
            // A damaged store behaves like an empty one; editing recreates it.
        }
    }

    private static Profile decode(JSONObject item) {
        try {
            byte[] secret = Base64.getDecoder().decode(item.optString("secret", ""));
            if (secret.length < MIN_SECRET) return null;
            String host = item.optString("host", "");
            if (host.isEmpty()) return null;
            return new Profile(item.optString("id"),
                item.optString("name", host),
                host,
                item.optInt("port", 9443),
                item.optString("sni", host),
                item.optString("marker", "sync/1"),
                secret,
                item.optString("caPem", null));
        } catch (Exception error) {
            return null;
        }
    }

    private static JSONObject encode(Profile profile) throws Exception {
        JSONObject item = new JSONObject();
        item.put("id", profile.id);
        item.put("name", profile.name);
        item.put("host", profile.host);
        item.put("port", profile.port);
        item.put("sni", profile.serverName);
        item.put("marker", profile.marker);
        item.put("secret", Base64.getEncoder().encodeToString(profile.secret));
        if (profile.caPem != null && !profile.caPem.isEmpty()) item.put("caPem", profile.caPem);
        return item;
    }

    private static synchronized void persist(Context context) {
        AtomicFile target = new AtomicFile(file(context));
        FileOutputStream output = null;
        try {
            JSONObject root = new JSONObject();
            root.put("active", activeId);
            JSONArray items = new JSONArray();
            for (Profile profile : profiles) items.put(encode(profile));
            root.put("profiles", items);
            // Recover the previous JSON database if a write is interrupted.
            // Never delete the last good copy after a failed rename.
            output = target.startWrite();
            output.write(root.toString().getBytes(StandardCharsets.UTF_8));
            target.finishWrite(output);
        } catch (Exception error) {
            if (output != null) target.failWrite(output);
            Log.e("Blade", "Unable to persist VPN profiles", error);
        }
    }

    public static synchronized List<Profile> all(Context context) {
        load(context);
        return new ArrayList<>(profiles);
    }

    public static synchronized Profile active(Context context) {
        load(context);
        for (Profile profile : profiles) {
            if (profile.id.equals(activeId)) return profile;
        }
        return profiles.isEmpty() ? null : profiles.get(0);
    }

    public static synchronized void setActive(Context context, String id) {
        load(context);
        activeId = id;
        persist(context);
    }

    public static synchronized Profile save(Context context, Profile profile) {
        load(context);
        profiles.removeIf(existing -> existing.id.equals(profile.id));
        profiles.add(profile);
        activeId = profile.id;
        persist(context);
        return profile;
    }

    public static synchronized boolean delete(Context context, String id) {
        load(context);
        boolean removed = profiles.removeIf(profile -> profile.id.equals(id));
        if (removed) {
            if (id.equals(activeId)) activeId = profiles.isEmpty() ? null : profiles.get(0).id;
            persist(context);
        }
        return removed;
    }

    /** Parse an sslim://host:port?sni=...&k=BASE64&u=marker&ca=BASE64&n=label link into a profile. */
    public static Profile fromUri(String uri) throws Exception {
        if (!uri.startsWith("sslim://")) throw new IllegalArgumentException("Ожидается ссылка sslim://");
        java.net.URI parsed = new java.net.URI(uri);
        String host = parsed.getHost();
        if (host == null || host.isEmpty()) throw new IllegalArgumentException("В ссылке нет адреса сервера");
        int port = parsed.getPort() == -1 ? 9443 : parsed.getPort();
        String sni = host;
        String marker = "sync/1";
        byte[] secret = null;
        String label = null;
        String caPem = null;
        for (String pair : (parsed.getRawQuery() == null ? "" : parsed.getRawQuery()).split("&")) {
            int eq = pair.indexOf('=');
            if (eq <= 0) continue;
            String key = pair.substring(0, eq);
            String value = pair.substring(eq + 1);
            try {
                value = java.net.URLDecoder.decode(value.replace("+", "%2B"), StandardCharsets.UTF_8.name());
            } catch (Exception ignored) {
                // Keep the raw value when it is not percent-encoded.
            }
            if (key.equals("k")) secret = Base64.getMimeDecoder().decode(value);
            else if (key.equals("sni")) sni = value;
            else if (key.equals("n")) label = value;
            else if (key.equals("u")) marker = value.isEmpty() ? "sync/1" : value;
            else if (key.equals("ca")) {
                try {
                    caPem = new String(Base64.getMimeDecoder().decode(value), StandardCharsets.UTF_8);
                } catch (Exception ignored) {
                    // A malformed CA is ignored; the editor allows entering it manually.
                }
            }
        }
        if (secret == null || secret.length < MIN_SECRET) {
            throw new IllegalArgumentException("В ссылке нет секрета (k=) или он короче 32 байт");
        }
        String name = label == null || label.isEmpty() ? host : label;
        return new Profile(newId(), name, host, port, sni, marker, secret, caPem);
    }

    public static String newId() {
        return "p" + Long.toString(System.currentTimeMillis(), 36)
            + Integer.toString((int) (Math.random() * 1296), 36);
    }

    private static byte[] readAll(File source) throws Exception {
        // AtomicFile restores interrupted writes from its .bak copy here.
        // Reading the base file directly would miss an otherwise recoverable
        // profile database after a crash during the first rename.
        AtomicFile store = new AtomicFile(source);
        if (!source.exists() && !new File(source.getPath() + ".bak").exists()) return null;
        try (FileInputStream in = store.openRead()) {
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            byte[] buffer = new byte[4096];
            int count;
            while ((count = in.read(buffer)) != -1) bytes.write(buffer, 0, count);
            return bytes.toByteArray();
        }
    }

}
