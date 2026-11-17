package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

/** Two-level settings: profiles and real application preferences. */
public final class SettingsActivity extends Activity {
    private static final int BG = 0xff0b121a;
    private static final int SURFACE = 0xff111e2a;
    private static final int BORDER = 0xff263746;
    private static final int WHITE = 0xffeff9f6;
    private static final int MUTED = 0xffacbfcc;
    private static final int MINT = 0xffb7f4cd;
    private static final Typeface BOLD = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private boolean showingOptions;

    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        getWindow().setStatusBarColor(BG);
        getWindow().setNavigationBarColor(BG);
        showingOptions = savedInstanceState != null && savedInstanceState.getBoolean("options");
        render();
    }

    @Override protected void onSaveInstanceState(Bundle out) {
        out.putBoolean("options", showingOptions);
        super.onSaveInstanceState(out);
    }

    @Override public void onBackPressed() {
        if (showingOptions) {
            showingOptions = false;
            render();
        } else {
            super.onBackPressed();
        }
    }

    private int dp(float value) {
        return (int) (value * getResources().getDisplayMetrics().density + 0.5f);
    }

    private TextView text(String value, int sp, int color, boolean bold) {
        TextView label = new TextView(this);
        label.setText(value);
        label.setTextSize(sp);
        label.setTextColor(color);
        if (bold) label.setTypeface(BOLD);
        return label;
    }

    private LinearLayout column() {
        LinearLayout view = new LinearLayout(this);
        view.setOrientation(LinearLayout.VERTICAL);
        return view;
    }

    private GradientDrawable surface() {
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(SURFACE);
        bg.setCornerRadius(dp(17));
        bg.setStroke(dp(1), BORDER);
        return bg;
    }

    private void add(LinearLayout parent, View child, int top) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.topMargin = dp(top);
        parent.addView(child, params);
    }

    private void render() {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(BG);
        scroll.setVerticalScrollBarEnabled(false);
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets bars = insets.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
                return insets;
            });
        }
        LinearLayout content = column();
        content.setPadding(dp(24), dp(22), dp(24), dp(26));
        scroll.addView(content);
        setContentView(scroll);

        TextView back = text("‹", 36, MINT, false);
        back.setGravity(Gravity.CENTER_VERTICAL);
        back.setContentDescription("Назад");
        back.setOnClickListener(view -> onBackPressed());
        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        header.addView(back, new LinearLayout.LayoutParams(dp(42), dp(48)));
        header.addView(text(showingOptions ? "Настройки приложения" : "Настройки",
            21, WHITE, true));
        add(content, header, 0);

        if (!showingOptions) {
            add(content, text("Управление Blade", 13, MUTED, false), 14);
            add(content, item("Профили", "Серверы, импорт, выбор и редактирование", () ->
                startActivity(new Intent(this, ProfilesActivity.class))), 22);
            add(content, item("Настройки приложения",
                "Защита соединения, DNS и диагностика", () -> {
                    showingOptions = true;
                    render();
                }), 12);
        } else {
            add(content, text("СОЕДИНЕНИЕ", 12, MINT, true), 20);
            add(content, toggle("Автоматически переподключаться",
                "Повторять подключение после сетевого обрыва",
                BladeOptions.autoReconnect(this),
                checked -> BladeOptions.setAutoReconnect(this, checked)), 10);

            add(content, item("DNS-серверы",
                BladeOptions.dnsName(this) + " · применяется после переподключения",
                this::chooseDns), 12);

            add(content, text("БЕЗОПАСНОСТЬ", 12, MINT, true), 24);
            add(content, item("Kill switch",
                "Системная блокировка трафика без VPN — настроить",
                this::showKillSwitchGuide), 10);
            add(content, text("Для полной защиты Android должен включить "
                + "«Постоянная VPN» и «Блокировать подключения без VPN». "
                + "Blade не может включить эти переключатели самостоятельно "
                + "и не показывает непроверенный статус как «Включено».",
                12, MUTED, false), 10);

            add(content, text("ДИАГНОСТИКА", 12, MINT, true), 24);
            add(content, toggle("Журнал производительности",
                "Показывать BladePerf в logcat для поиска проблем со скоростью",
                BladeOptions.diagnostics(this),
                checked -> BladeOptions.setDiagnostics(this, checked)), 10);
        }
    }

    private View item(String title, String subtitle, Runnable click) {
        LinearLayout row = column();
        row.setPadding(dp(17), dp(15), dp(17), dp(15));
        row.setBackground(surface());
        row.setClickable(true);
        row.setFocusable(true);
        row.setContentDescription(title + ". " + subtitle);
        row.setOnClickListener(view -> click.run());
        add(row, text(title + "  ›", 16, WHITE, true), 0);
        add(row, text(subtitle, 12, MUTED, false), 5);
        return row;
    }

    private interface ToggleAction { void changed(boolean value); }

    private View toggle(String title, String subtitle, boolean current, ToggleAction action) {
        LinearLayout row = new LinearLayout(this);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(dp(16), dp(15), dp(16), dp(15));
        row.setBackground(surface());
        LinearLayout words = column();
        add(words, text(title, 15, WHITE, true), 0);
        add(words, text(subtitle, 12, MUTED, false), 5);
        row.addView(words, new LinearLayout.LayoutParams(0, -2, 1f));
        Switch control = new Switch(this);
        control.setContentDescription(title);
        control.setChecked(current);
        control.setOnCheckedChangeListener((button, checked) -> action.changed(checked));
        row.addView(control);
        row.setOnClickListener(view -> control.setChecked(!control.isChecked()));
        return row;
    }

    private void chooseDns() {
        String[] names = {"Cloudflare — 1.1.1.1", "Google — 8.8.8.8", "Quad9 — 9.9.9.9"};
        String[] values = {BladeOptions.DNS_CLOUDFLARE,
            BladeOptions.DNS_GOOGLE, BladeOptions.DNS_QUAD9};
        String current = BladeOptions.dns(this);
        int selected = 0;
        for (int i = 0; i < values.length; i++) {
            if (values[i].equals(current)) selected = i;
        }
        new AlertDialog.Builder(this)
            .setTitle("DNS-серверы")
            .setSingleChoiceItems(names, selected, (dialog, which) -> {
                BladeOptions.setDns(this, values[which]);
                dialog.dismiss();
                render();
            })
            .setNegativeButton("Отмена", null)
            .show();
    }

    private void showKillSwitchGuide() {
        new AlertDialog.Builder(this)
            .setTitle("Kill switch — системная защита")
            .setMessage("Android обеспечивает надёжную блокировку трафика даже при "
                + "сбое Blade. Откройте настройки VPN, нажмите на Blade "
                + "и включите:\n\n1. «Постоянная VPN» (Always-on VPN)"
                + "\n2. «Блокировать подключения без VPN» (Block connections without VPN)"
                + "\n\nЕсли VPN оборвётся, Android будет блокировать доступ "
                + "в интернет, пока туннель не восстановится. "
                + "Без второго системного переключателя kill switch НЕ активен.")
            .setPositiveButton("Настройки VPN", (dialog, which) -> {
                try {
                    startActivity(new Intent(Settings.ACTION_VPN_SETTINGS));
                } catch (ActivityNotFoundException error) {
                    startActivity(new Intent(Settings.ACTION_WIRELESS_SETTINGS));
                }
            })
            .setNegativeButton("Закрыть", null)
            .show();
    }
}
