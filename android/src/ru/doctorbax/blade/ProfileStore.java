package ru.doctorbax.blade;

import android.content.Context;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
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
        public final List<String> excludedPackages;

        Profile(String id, String name, String host, int port, String serverName,
                String marker, byte[] secret, String caPem) {
            this(id, name, host, port, serverName, marker, secret, caPem, null);
        }

        Profile(String id, String name, String host, int port, String serverName,
                String marker, byte[] secret, String caPem, List<String> excludedPackages) {
            this.id = id;
            this.name = name;
            this.host = host;
            this.port = port;
            this.serverName = serverName;
            this.marker = marker;
            this.secret = secret;
            this.caPem = caPem;
            if (excludedPackages == null) {
                this.excludedPackages = null;
            } else {
                List<String> valid = new ArrayList<>();
                for (String packageName : excludedPackages) {
                    if (packageName != null && packageName.matches(
                            "[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+")
                            && !valid.contains(packageName)) {
                        valid.add(packageName);
                        if (valid.size() == 100) break;
                    }
                }
                this.excludedPackages = java.util.Collections.unmodifiableList(valid);
            }
        }

        public String uri() {
            return "sslim://" + host + ":" + port + "?sni=" + serverName
                + "&k=" + Base64.getEncoder().encodeToString(secret) + "&n=" + name;
        }
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
            List<String> excluded = new ArrayList<>();
            JSONArray packages = item.optJSONArray("excludedPackages");
            for (int i = 0; packages != null && i < packages.length(); i++) {
                String name = packages.optString(i, "");
                if (!name.isEmpty()) excluded.add(name);
            }
            return new Profile(item.optString("id"),
                item.optString("name", host),
                host,
                item.optInt("port", 9443),
                item.optString("sni", host),
                item.optString("marker", "sync/1"),
                secret,
                item.optString("caPem", null), excluded);
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
        item.put("excludedPackages", new JSONArray(profile.excludedPackages));
        return item;
    }

    private static synchronized void persist(Context context) {
        try {
            JSONObject root = new JSONObject();
            root.put("active", activeId);
            JSONArray items = new JSONArray();
            for (Profile profile : profiles) items.put(encode(profile));
            root.put("profiles", items);
            File target = file(context);
            File temporary = new File(target.getParentFile(), FILE + ".tmp");
            try (FileOutputStream out = new FileOutputStream(temporary)) {
                out.write(root.toString().getBytes(StandardCharsets.UTF_8));
            }
            if (!temporary.renameTo(target)) {
                target.delete();
                temporary.renameTo(target);
            }
        } catch (Exception ignored) {
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
        if (profile.excludedPackages == null) {
            List<String> excluded = new ArrayList<>();
            for (Profile existing : profiles) {
                if (existing.id.equals(profile.id) && existing.excludedPackages != null) {
                    excluded.addAll(existing.excludedPackages);
                    break;
                }
            }
            profile = new Profile(profile.id, profile.name, profile.host, profile.port,
                profile.serverName, profile.marker, profile.secret, profile.caPem, excluded);
        }
        final String savedId = profile.id;
        profiles.removeIf(existing -> existing.id.equals(savedId));
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
        if (!source.exists()) return null;
        try (FileInputStream in = new FileInputStream(source)) {
            byte[] data = new byte[(int) source.length()];
            int read = 0;
            while (read < data.length) {
                int chunk = in.read(data, read, data.length - read);
                if (chunk < 0) break;
                read += chunk;
            }
            return java.util.Arrays.copyOf(data, read);
        }
    }
}
