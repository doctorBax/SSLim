package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.util.Locale;

public final class MainActivity extends Activity {
    private static final int BACKGROUND = 0xff0b121a;
    private static final int SURFACE = 0xff111e2a;
    private static final int BORDER = 0xff263746;
    private static final int WHITE = 0xffeff9f6;
    private static final int MUTED = 0xffacbfcc;
    private static final int MINT = 0xffb7f4cd;
    private static final int CORAL = 0xffffb4a8;
    private static final int REQUEST_VPN = 1;
    private static final int REQUEST_EDIT = 2;
    private static final Typeface MEDIUM = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private static final Typeface BOLD = Typeface.create("sans-serif", Typeface.BOLD);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Button button;
    private TextView chip;
    private TextView headline;
    private TextView details;
    private TextView profileName;
    private TextView profileEndpoint;
    private ImageView heroArrow;
    private Orbit orbit;
    private String shownStatus;
    private boolean shownRunning;
    private TextView throughputDown;
    private TextView throughputUp;
    private TextView latency;
    private BladeVpn.Traffic currentTraffic;
    private long sampledAt;
    private long previousDown;
    private long previousUp;

    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            renderProfile();
            renderStatus();
            renderTraffic();
            handler.postDelayed(this, 300);
        }
    };

    @Override public void onCreate(Bundle savedState) {
        super.onCreate(savedState);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        getWindow().setStatusBarColor(BACKGROUND);
        getWindow().setNavigationBarColor(BACKGROUND);
        getWindow().getDecorView().setSystemUiVisibility(0);

        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(BACKGROUND);
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
        content.setPadding(dp(24), dp(24), dp(24), dp(22));
        content.setGravity(Gravity.CENTER_VERTICAL);
        scroll.addView(content);

        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        ImageView brand = arrow(40, 20, MINT, SURFACE, 12);
        header.addView(brand, new LinearLayout.LayoutParams(dp(40), dp(40)));
        TextView name = label("Blade", 20, WHITE, BOLD);
        LinearLayout.LayoutParams brandText = new LinearLayout.LayoutParams(0, -2, 1f);
        brandText.setMarginStart(dp(12));
        header.addView(name, brandText);
        TextView version = label("клиент SSLim", 11, MUTED, MEDIUM);
        version.setPadding(dp(10), dp(6), dp(10), dp(6));
        version.setBackground(surface(BACKGROUND, 12, BORDER));
        header.addView(version);
        add(content, header, 0);

        FrameLayout hero = new FrameLayout(this);
        GradientDrawable panel = new GradientDrawable(
            GradientDrawable.Orientation.TL_BR, new int[]{0xff142a37, 0xff0f1b28});
        panel.setCornerRadius(dp(28));
        panel.setStroke(dp(1), BORDER);
        hero.setBackground(panel);
        hero.setClipToOutline(true);
        orbit = new Orbit();
        hero.addView(orbit, new FrameLayout.LayoutParams(-1, -1));
        heroArrow = arrow(82, 40, BACKGROUND, MINT, 24);
        hero.addView(heroArrow, new FrameLayout.LayoutParams(dp(82), dp(82), Gravity.CENTER));
        int heroHeight = getResources().getConfiguration().screenHeightDp < 680 ? 160 : 214;
        LinearLayout.LayoutParams heroSize = new LinearLayout.LayoutParams(-1, dp(heroHeight));
        heroSize.topMargin = dp(22);
        content.addView(hero, heroSize);

        LinearLayout summary = column();
        summary.setGravity(Gravity.CENTER_HORIZONTAL);
        chip = label("", 12, MINT, MEDIUM);
        chip.setGravity(Gravity.CENTER);
        chip.setPadding(dp(13), dp(6), dp(13), dp(6));
        chip.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        summary.addView(chip, new LinearLayout.LayoutParams(-2, -2));

        headline = label("", 30, WHITE, BOLD);
        headline.setGravity(Gravity.CENTER);
        add(summary, headline, 11);
        details = label("", 14, MUTED, null);
        details.setGravity(Gravity.CENTER);
        details.setLineSpacing(dp(3), 1);
        add(summary, details, 6);
        add(content, summary, 20);

        // Active profile card: tap to manage profiles.
        LinearLayout endpoint = new LinearLayout(this);
        endpoint.setGravity(Gravity.CENTER_VERTICAL);
        endpoint.setBackground(surface(SURFACE, 20, BORDER));
        endpoint.setPadding(dp(16), dp(12), dp(16), dp(12));
        endpoint.setOnClickListener(view -> showProfilesDialog());
        endpoint.setClickable(true);
        LinearLayout server = column();
        server.addView(label("ПРОФИЛЬ", 10, MUTED, MEDIUM));
        profileName = label("—", 16, WHITE, BOLD);
        add(server, profileName, 3);
        profileEndpoint = label("Нажмите, чтобы выбрать сервер", 12, MUTED, null);
        add(server, profileEndpoint, 2);
        endpoint.addView(server, new LinearLayout.LayoutParams(0, -2, 1));
        TextView manage = label("Выбрать ▸", 13, MINT, MEDIUM);
        manage.setPadding(dp(10), dp(6), dp(10), dp(6));
        manage.setBackground(surface(BACKGROUND, 12, BORDER));
        endpoint.addView(manage);
        add(content, endpoint, 22);

        LinearLayout actions = new LinearLayout(this);
        Button paste = pill("Вставить sslim://");
        paste.setOnClickListener(view -> importFromClipboard());
        actions.addView(paste, new LinearLayout.LayoutParams(0, -2, 1f));
        LinearLayout.LayoutParams gap = new LinearLayout.LayoutParams(dp(10), 0);
        actions.addView(new View(this), gap);
        Button create = pill("Создать профиль");
        create.setOnClickListener(view -> openEditor(null));
        actions.addView(create, new LinearLayout.LayoutParams(0, -2, 1f));
        add(content, actions, 12);

        button = new Button(this);
        button.setAllCaps(false);
        button.setTextSize(17);
        button.setTypeface(BOLD);
        button.setMinHeight(dp(56));
        button.setStateListAnimator(null);
        button.setOnClickListener(view -> toggleConnection());
        add(content, button, 16);

        LinearLayout stats = new LinearLayout(this);
        stats.setBackground(surface(SURFACE, 20, BORDER));
        stats.setPadding(dp(8), dp(12), dp(8), dp(12));
        throughputDown = addStat(stats, "Загрузка");
        throughputUp = addStat(stats, "Отдача");
        latency = addStat(stats, "Пинг до VPS");
        add(content, stats, 14);

        setContentView(scroll);
        handleIncomingIntent(getIntent());
        renderProfile();
    }

    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIncomingIntent(intent);
    }

    private void handleIncomingIntent(Intent intent) {
        if (intent == null || intent.getData() == null) return;
        importUri(intent.getData().toString());
    }

    private void importFromClipboard() {
        ClipboardManager clipboard = getSystemService(ClipboardManager.class);
        CharSequence text = clipboard == null || clipboard.getPrimaryClip() == null
            || clipboard.getPrimaryClip().getItemAt(0) == null ? null :
            clipboard.getPrimaryClip().getItemAt(0).getText();
        if (text == null) {
            toast("Буфер обмена пуст");
            return;
        }
        importUri(text.toString().trim());
    }

    private void importUri(String source) {
        try {
            ProfileStore.Profile imported = ProfileStore.fromUri(source.replaceAll("\\s+", ""));
            ProfileStore.save(this, imported);
            toast("Профиль «" + imported.name + "» добавлен");
        } catch (Exception error) {
            toast("Не удалось импортировать: " + error.getMessage());
        }
    }

    private void openEditor(String id) {
        Intent editor = new Intent(this, ProfileEditorActivity.class);
        editor.putExtra("id", id);
        startActivityForResult(editor, REQUEST_EDIT);
    }

    private void showProfilesDialog() {
        java.util.List<ProfileStore.Profile> profiles = ProfileStore.all(this);
        ProfileStore.Profile active = ProfileStore.active(this);
        if (profiles.isEmpty()) {
            openEditor(null);
            return;
        }
        LinearLayout list = column();
        for (final ProfileStore.Profile profile : profiles) {
            LinearLayout row = new LinearLayout(this);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(dp(16), dp(12), dp(16), dp(12));
            GradientDrawable background = surface(
                active != null && active.id.equals(profile.id) ? 0xff17342a : SURFACE, 16, BORDER);
            row.setBackground(background);
            LinearLayout text = column();
            text.addView(label(profile.name, 15, WHITE, BOLD));
            add(text, label(profile.host + ":" + profile.port + " · " + profile.serverName, 12, MUTED, null), 2);
            row.addView(text, new LinearLayout.LayoutParams(0, -2, 1f));
            TextView edit = label("✎", 16, MINT, MEDIUM);
            edit.setPadding(dp(12), dp(8), dp(4), dp(8));
            edit.setOnClickListener(view -> { dialogDismiss(row); openEditor(profile.id); });
            row.addView(edit);
            TextView remove = label("✕", 16, CORAL, MEDIUM);
            remove.setPadding(dp(10), dp(8), dp(4), dp(8));
            remove.setOnClickListener(view -> confirmDelete(profile));
            row.addView(remove);
            row.setOnClickListener(view -> {
                ProfileStore.setActive(this, profile.id);
                dialogDismiss(row);
                renderProfile();
            });
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
            params.topMargin = dp(8);
            list.addView(row, params);
        }
        ScrollView listScroll = new ScrollView(this);
        listScroll.addView(list);
        listScroll.setPadding(dp(16), dp(16), dp(16), dp(8));
        currentDialog = new AlertDialog.Builder(this)
            .setTitle("Профили")
            .setView(listScroll)
            .setNegativeButton("Закрыть", null)
            .setPositiveButton("Новый профиль", (dialog, which) -> openEditor(null))
            .show();
    }

    private AlertDialog currentDialog;

    private void dialogDismiss(View view) {
        if (currentDialog != null) currentDialog.dismiss();
    }

    private void confirmDelete(ProfileStore.Profile profile) {
        new AlertDialog.Builder(this)
            .setTitle("Удалить «" + profile.name + "»?")
            .setPositiveButton("Удалить", (dialog, which) -> {
                ProfileStore.delete(this, profile.id);
                renderProfile();
                if (currentDialog != null) currentDialog.dismiss();
            })
            .setNegativeButton("Отмена", null)
            .show();
    }

    private Button pill(String text) {
        Button view = new Button(this);
        view.setText(text);
        view.setAllCaps(false);
        view.setTextSize(14);
        view.setTypeface(MEDIUM);
        view.setTextColor(WHITE);
        view.setMinHeight(dp(46));
        view.setStateListAnimator(null);
        view.setBackground(new RippleDrawable(
            ColorStateList.valueOf(0x30ffffff), surface(SURFACE, 16, BORDER), null));
        return view;
    }

    private TextView addStat(LinearLayout stats, String title) {
        LinearLayout cell = column();
        cell.setGravity(Gravity.CENTER_HORIZONTAL);
        TextView description = label(title, 11, MUTED, MEDIUM);
        description.setGravity(Gravity.CENTER);
        add(cell, description, 0);
        TextView value = label("—", 13, WHITE, BOLD);
        value.setGravity(Gravity.CENTER);
        add(cell, value, 4);
        stats.addView(cell, new LinearLayout.LayoutParams(0, -2, 1f));
        return value;
    }

    private void renderProfile() {
        ProfileStore.Profile profile = ProfileStore.active(this);
        if (profile == null) {
            profileName.setText("нет профиля");
            profileEndpoint.setText("Создайте или вставьте sslim:// ссылку");
        } else {
            profileName.setText(profile.name);
            profileEndpoint.setText(profile.host + ":" + profile.port + " · SNI " + profile.serverName);
        }
    }

    private void renderTraffic() {
        BladeVpn.Traffic counters = BladeVpn.traffic;
        if (!"VPN подключён".equals(BladeVpn.status)) {
            if (sampledAt != 0) {
                throughputDown.setText("—");
                throughputUp.setText("—");
                latency.setText("—");
                sampledAt = 0;
            }
            return;
        }
        long now = SystemClock.elapsedRealtime();
        long down = counters.download.get();
        long up = counters.upload.get();
        long elapsed = now - sampledAt;
        if (currentTraffic != counters || sampledAt == 0) {
            currentTraffic = counters;
        } else if (elapsed >= 1000) {
            throughputDown.setText(rate(down - previousDown, elapsed));
            throughputUp.setText(rate(up - previousUp, elapsed));
        } else {
            return;
        }
        latency.setText(counters.rttMs >= 0 && now - counters.rttSampleTime < 10000 ?
            counters.rttMs + " мс" : "—");
        sampledAt = now;
        previousDown = down;
        previousUp = up;
    }

    private String rate(long bytes, long ms) {
        return String.format(Locale.getDefault(), "%.1f Мбит/с", bytes * 0.008 / ms);
    }

    private void renderStatus() {
        String status = BladeVpn.status;
        boolean running = BladeVpn.running;
        if (status.equals(shownStatus) && running == shownRunning) return;
        shownStatus = status;
        shownRunning = running;

        boolean connected = "VPN подключён".equals(status);
        boolean starting = "Подключение…".equals(status);
        boolean stopping = "Отключение…".equals(status);
        boolean failed = !connected && !starting && !stopping && !"Отключено".equals(status);
        int tone = failed ? CORAL : connected || starting ? MINT : MUTED;
        chip.setText(connected ? "В СЕТИ" : starting ? "СОЕДИНЕНИЕ" : stopping ? "ОТКЛЮЧЕНИЕ" : failed ? "СБОЙ" : "НЕ АКТИВЕН");
        chip.setTextColor(tone);
        chip.setBackground(surface(connected ? 0xff17342a : SURFACE, 20, BORDER));

        headline.setText(connected ? "VPN подключён" : starting ? "Подключаемся…" :
            stopping ? "Отключаемся…" : failed ? "Соединение потеряно" : "VPN выключен");
        details.setText(connected ? "IPv4-трафик проходит через туннель." :
            starting ? "Устанавливаем защищённое соединение." :
            stopping ? "Завершаем текущий сеанс." : failed ? status : "Подключитесь, чтобы включить туннель.");

        orbit.active = connected || starting;
        orbit.invalidate();
        heroArrow.setBackground(surface(connected ? 0xff86efb3 : MINT, 24, 0));
        button.setText(connected ? "Отключить VPN" : starting ? "Отменить" : stopping ? "Отключаем…" : "Подключить VPN");
        button.setEnabled(!stopping);
        button.setTextColor(running ? WHITE : BACKGROUND);
        button.setBackground(new RippleDrawable(
            ColorStateList.valueOf(running ? 0x30ffffff : 0x280b121a),
            surface(running ? SURFACE : MINT, 18, running ? BORDER : 0), null));
    }

    private void toggleConnection() {
        if (BladeVpn.running) {
            startService(new Intent(this, BladeVpn.class).setAction(BladeVpn.DISCONNECT));
            return;
        }
        if (ProfileStore.active(this) == null) {
            toast("Сначала добавьте профиль сервера");
            openEditor(null);
            return;
        }
        Intent permission = VpnService.prepare(this);
        if (permission == null) connect();
        else startActivityForResult(permission, REQUEST_VPN);
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }

    private LinearLayout column() {
        LinearLayout view = new LinearLayout(this);
        view.setOrientation(LinearLayout.VERTICAL);
        return view;
    }

    private TextView label(String value, int size, int color, Typeface font) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        if (font != null) view.setTypeface(font);
        return view;
    }

    private void add(LinearLayout parent, View view, int margin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.topMargin = dp(margin);
        parent.addView(view, params);
    }

    private ImageView arrow(int box, int size, int tint, int background, int radius) {
        ImageView icon = new ImageView(this);
        icon.setImageResource(R.drawable.ic_arrow_right);
        icon.setColorFilter(tint);
        icon.setScaleType(ImageView.ScaleType.FIT_CENTER);
        icon.setBackground(surface(background, radius, 0));
        int padding = dp((box - size) / 2f);
        icon.setPadding(padding, padding, padding, padding);
        icon.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        return icon;
    }

    private GradientDrawable surface(int color, int radius, int stroke) {
        GradientDrawable value = new GradientDrawable();
        value.setColor(color);
        value.setCornerRadius(dp(radius));
        if (stroke != 0) value.setStroke(dp(1), stroke);
        return value;
    }

    private int dp(float value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private final class Orbit extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        boolean active;

        Orbit() {
            super(MainActivity.this);
            setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        }

        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            float x = getWidth() / 2f;
            float y = getHeight() / 2f;
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(dp(1));
            paint.setColor(active ? 0x427fe4b0 : 0x315c91aa);
            for (int radius = 57; radius <= 125; radius += 17) {
                canvas.drawCircle(x, y, dp(radius), paint);
            }
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(active ? MINT : 0xff759eac);
            canvas.drawCircle(x + dp(69), y - dp(27), dp(3), paint);
            canvas.drawCircle(x - dp(90), y + dp(14), dp(2), paint);
        }
    }

    @Override protected void onStart() {
        super.onStart();
        refresh.run();
    }

    @Override protected void onStop() {
        handler.removeCallbacks(refresh);
        super.onStop();
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == REQUEST_VPN && result == RESULT_OK) connect();
        if (request == REQUEST_EDIT) renderProfile();
    }

    private void connect() {
        ProfileStore.Profile profile = ProfileStore.active(this);
        if (profile == null) return;
        Intent launch = new Intent(this, BladeVpn.class)
            .putExtra("name", profile.name)
            .putExtra("host", profile.host)
            .putExtra("port", profile.port)
            .putExtra("sni", profile.serverName)
            .putExtra("secretBase64",
                android.util.Base64.encodeToString(profile.secret, android.util.Base64.NO_WRAP));
        if (profile.caPem != null && !profile.caPem.isEmpty()) {
            launch.putExtra("caPem", profile.caPem);
        }
        startForegroundService(launch);
    }
}
