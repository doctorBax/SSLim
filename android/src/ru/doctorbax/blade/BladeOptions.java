package ru.doctorbax.blade;

import android.content.Context;
import android.content.SharedPreferences;

/** Non-secret app preferences, independent of the existing profile store. */
final class BladeOptions {
    static final String DNS_CLOUDFLARE = "cloudflare";
    static final String DNS_GOOGLE = "google";
    static final String DNS_QUAD9 = "quad9";
    private static final String FILE = "blade-options";

    private BladeOptions() {}

    private static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    static boolean autoReconnect(Context context) {
        return prefs(context).getBoolean("autoReconnect", true);
    }

    static void setAutoReconnect(Context context, boolean value) {
        prefs(context).edit().putBoolean("autoReconnect", value).apply();
    }

    static boolean diagnostics(Context context) {
        return prefs(context).getBoolean("diagnostics", false);
    }

    static void setDiagnostics(Context context, boolean value) {
        prefs(context).edit().putBoolean("diagnostics", value).apply();
    }

    static String dns(Context context) {
        String value = prefs(context).getString("dns", DNS_CLOUDFLARE);
        if (DNS_GOOGLE.equals(value) || DNS_QUAD9.equals(value)) return value;
        return DNS_CLOUDFLARE;
    }

    static void setDns(Context context, String value) {
        prefs(context).edit().putString("dns", value).apply();
    }

    static String[] dnsServers(Context context) {
        switch (dns(context)) {
            case DNS_GOOGLE: return new String[]{"8.8.8.8", "8.8.4.4"};
            case DNS_QUAD9: return new String[]{"9.9.9.9", "149.112.112.112"};
            default: return new String[]{"1.1.1.1", "1.0.0.1"};
        }
    }

    static String dnsName(Context context) {
        switch (dns(context)) {
            case DNS_GOOGLE: return "Google";
            case DNS_QUAD9: return "Quad9";
            default: return "Cloudflare";
        }
    }
}
