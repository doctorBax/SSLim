package ru.doctorbax.blade;

import android.app.Activity;
import android.content.Context;
import android.content.res.Configuration;
import android.graphics.Color;
import android.view.View;

/** Shared Android Blade appearance and accessibility preferences. */
final class BladeAppearance {
    static final String SYSTEM = "system";
    static final String DARK = "dark";
    static final String LIGHT = "light";
    static final String MINT = "mint";
    static final String BLUE = "blue";
    static final String LAVENDER = "lavender";
    static final String AMBER = "amber";
    static final String CUSTOM = "custom";
    static final String ROUND = "round";
    static final String SQUARE = "square";
    static final String[] ANIMATION_NAMES = {"Orbit", "Aurora", "Data Stream", "Spectrum", "Crystal"};
    static final int DEFAULT_CUSTOM = 0xff71c9c0;

    private BladeAppearance() {}

    static String mode(Context c) {
        String value = c.getSharedPreferences("blade-appearance", 0).getString("mode", SYSTEM);
        return LIGHT.equals(value) || DARK.equals(value) ? value : SYSTEM;
    }

    static void setMode(Context c, String mode) {
        c.getSharedPreferences("blade-appearance", 0).edit().putString("mode", mode).apply();
    }

    static boolean isDark(Context c) {
        if (DARK.equals(mode(c))) return true;
        if (LIGHT.equals(mode(c))) return false;
        return (c.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
            == Configuration.UI_MODE_NIGHT_YES;
    }

    static String accentName(Context c) {
        String value = c.getSharedPreferences("blade-appearance", 0).getString("accent", MINT);
        switch (value) {
            case BLUE: case LAVENDER: case AMBER: case CUSTOM: return value;
            default: return MINT;
        }
    }

    static void setAccentName(Context c, String value) {
        c.getSharedPreferences("blade-appearance", 0).edit().putString("accent", value).apply();
    }

    static int customColor(Context c) {
        return 0xff000000 | c.getSharedPreferences("blade-appearance", 0)
            .getInt("customColor", DEFAULT_CUSTOM);
    }

    static void setCustomColor(Context c, int color) {
        c.getSharedPreferences("blade-appearance", 0).edit()
            .putInt("customColor", 0xff000000 | color).apply();
    }

    static int accent(Context c) {
        boolean dark = isDark(c);
        int color;
        switch (accentName(c)) {
            case BLUE: color = dark ? 0xff8cbdf0 : 0xff2869ad; break;
            case LAVENDER: color = dark ? 0xffb5a2e8 : 0xff7350b1; break;
            case AMBER: color = dark ? 0xffe9c68a : 0xff97621a; break;
            case CUSTOM: color = customColor(c); break;
            default: color = dark ? 0xffb7f4cd : 0xff247d5e;
        }
        // Custom HEX is respected exactly; onAccent chooses readable arrow ink.
        return color;
    }

    static int onAccent(Context c) {
        int color = accent(c);
        int r = Color.red(color), g = Color.green(color), b = Color.blue(color);
        return (r * 299 + g * 587 + b * 114) / 1000 >= 150 ? 0xff11232b : 0xffffffff;
    }

    static int background(Context c) { return isDark(c) ? 0xff0b121a : 0xfff5f7fa; }
    static int surface(Context c) { return isDark(c) ? 0xff111e2a : 0xffffffff; }
    static int border(Context c) { return isDark(c) ? 0xff263746 : 0xffdbe3e8; }
    static int text(Context c) { return isDark(c) ? 0xffeff9f6 : 0xff192a34; }
    static int muted(Context c) { return isDark(c) ? 0xffacbfcc : 0xff526675; }
    static int danger(Context c) { return isDark(c) ? 0xffffb4a8 : 0xffaf4a4d; }
    static int selected(Context c) {
        int base = surface(c), tone = accent(c);
        float blend = isDark(c) ? 0.17f : 0.11f;
        int r = Math.round(Color.red(base) * (1f-blend) + Color.red(tone) * blend);
        int g = Math.round(Color.green(base) * (1f-blend) + Color.green(tone) * blend);
        int b = Math.round(Color.blue(base) * (1f-blend) + Color.blue(tone) * blend);
        return Color.rgb(r, g, b);
    }
    static int heroTop(Context c) { return isDark(c) ? 0xff142a37 : 0xffedf4f7; }
    static int heroBottom(Context c) { return isDark(c) ? 0xff0f1b28 : 0xffe6eef3; }
    static int heroPressedTop(Context c) { return isDark(c) ? 0xff1b3348 : 0xffdeedf2; }
    static int heroPressedBottom(Context c) { return isDark(c) ? 0xff122333 : 0xffdbe8ee; }

    static int animation(Context c) {
        int index = c.getSharedPreferences("blade-appearance", 0).getInt("animation", 0);
        return index >= 0 && index < ANIMATION_NAMES.length ? index : 0;
    }

    static void setAnimation(Context c, int index) {
        c.getSharedPreferences("blade-appearance", 0).edit().putInt("animation", index).apply();
    }

    static boolean square(Context c) {
        return c.getSharedPreferences("blade-appearance", 0).getBoolean("square", false);
    }

    static void setSquare(Context c, boolean enabled) {
        c.getSharedPreferences("blade-appearance", 0).edit().putBoolean("square", enabled).apply();
    }

    static boolean haptics(Context c) {
        return c.getSharedPreferences("blade-appearance", 0).getBoolean("haptics", true);
    }

    static void setHaptics(Context c, boolean enabled) {
        c.getSharedPreferences("blade-appearance", 0).edit().putBoolean("haptics", enabled).apply();
    }

    static void haptic(View view) {
        if (haptics(view.getContext())) {
            view.performHapticFeedback(android.view.HapticFeedbackConstants.VIRTUAL_KEY);
        }
    }

    static String stamp(Context c) {
        int night = c.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        return mode(c) + "|" + accentName(c) + "|" + customColor(c) + "|"
            + animation(c) + "|" + square(c) + "|" + night;
    }

    static void applyWindow(Activity activity) {
        boolean dark = isDark(activity);
        activity.getWindow().setStatusBarColor(background(activity));
        activity.getWindow().setNavigationBarColor(background(activity));
        int flags = dark ? 0 : View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
            | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        activity.getWindow().getDecorView().setSystemUiVisibility(flags);
    }
}
