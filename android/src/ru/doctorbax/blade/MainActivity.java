package ru.doctorbax.blade;

import android.app.Activity;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RadialGradient;
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
    private static final int REQUEST_SETTINGS = 3;
    private static final Typeface MEDIUM = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private static final Typeface BOLD = Typeface.create("sans-serif", Typeface.BOLD);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private HeroView hero;
    private TextView chip;
    private TextView headline;
    private TextView details;
    private TextView activeProfileName;
    private TextView activeProfileAddress;
    private TextView activeProfileAction;
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
        scroll.addView(content, new ScrollView.LayoutParams(-1, -1));

        // Header: gear in the corner opens settings; profile work lives there.
        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView name = label("Blade", 20, WHITE, BOLD);
        header.addView(name, new LinearLayout.LayoutParams(0, -2, 1f));
        ImageView gear = iconButton(R.drawable.ic_gear, 40, MINT, SURFACE, 12);
        gear.setOnClickListener(view -> openSettings());
        gear.setContentDescription("Настройки");
        header.addView(gear, new LinearLayout.LayoutParams(dp(40), dp(40)));
        add(content, header, 0);
        addFlexibleSpace(content);

        // Hero panel is the connect button itself.
        FrameLayout heroPanel = new FrameLayout(this);
        GradientDrawable panel = new GradientDrawable(
            GradientDrawable.Orientation.TL_BR, new int[]{0xff142a37, 0xff0f1b28});
        panel.setCornerRadius(dp(28));
        panel.setStroke(dp(1), BORDER);
        GradientDrawable pressed = new GradientDrawable(
            GradientDrawable.Orientation.TL_BR, new int[]{0xff1b3348, 0xff122333});
        heroPanel.setBackground(new RippleDrawable(
            ColorStateList.valueOf(0x40b7f4cd), panel, pressed));
        heroPanel.setClickable(true);
        heroPanel.setOnClickListener(view -> toggleConnection());
        heroPanel.setContentDescription("Подключить или отключить VPN");
        hero = new HeroView(this);
        heroPanel.addView(hero, new FrameLayout.LayoutParams(-1, -1));
        ImageView centerArrow = arrow(82, 40, BACKGROUND, MINT, 24);
        heroPanel.addView(centerArrow, new FrameLayout.LayoutParams(dp(82), dp(82), Gravity.CENTER));
        int screenHeight = getResources().getConfiguration().screenHeightDp;
        int heroHeight = Math.max(168, Math.min(280, screenHeight / 3));
        LinearLayout.LayoutParams heroSize = new LinearLayout.LayoutParams(-1, dp(heroHeight));
        heroSize.topMargin = dp(22);
        content.addView(heroPanel, heroSize);

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
        addFlexibleSpace(content);

        LinearLayout stats = new LinearLayout(this);
        stats.setBackground(surface(SURFACE, 20, BORDER));
        stats.setPadding(dp(8), dp(12), dp(8), dp(12));
        throughputDown = addStat(stats, "Загрузка");
        throughputUp = addStat(stats, "Отдача");
        latency = addStat(stats, "Пинг до VPS");
        add(content, stats, 14);

        LinearLayout profileCard = new LinearLayout(this);
        profileCard.setGravity(Gravity.CENTER_VERTICAL);
        profileCard.setPadding(dp(16), dp(12), dp(16), dp(12));
        profileCard.setBackground(surface(SURFACE, 18, BORDER));
        profileCard.setClickable(true);
        profileCard.setForeground(new RippleDrawable(
            ColorStateList.valueOf(0x30ffffff), null, surface(SURFACE, 18, 0)));
        profileCard.setContentDescription("Профиль сервера. Открыть настройки");
        profileCard.setOnClickListener(view -> openSettings());
        LinearLayout profileText = column();
        profileText.addView(label("СЕРВЕР", 10, MUTED, MEDIUM));
        activeProfileName = label("", 15, WHITE, BOLD);
        add(profileText, activeProfileName, 3);
        activeProfileAddress = label("", 12, MUTED, null);
        add(profileText, activeProfileAddress, 2);
        profileCard.addView(profileText, new LinearLayout.LayoutParams(0, -2, 1f));
        activeProfileAction = label("Настроить  ›", 12, MINT, MEDIUM);
        profileCard.addView(activeProfileAction);
        add(content, profileCard, 12);

        setContentView(scroll);
        handleIncomingIntent(getIntent());
        renderProfile();
        renderStatus();
    }

    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIncomingIntent(intent);
        renderProfile();
    }

    /** sslim:// links (from a browser or chat) import the profile silently. */
    private void handleIncomingIntent(Intent intent) {
        if (intent == null || intent.getData() == null) return;
        android.util.Log.d("Blade", "incoming profile link, length " + intent.getData().toString().length());
        try {
            ProfileStore.Profile imported = ProfileStore.fromUri(
                intent.getData().toString().replaceAll("\\s+", ""));
            ProfileStore.save(this, imported);
            toast("Профиль «" + imported.name + "» добавлен");
        } catch (Exception error) {
            android.util.Log.w("Blade", "import failed", error);
            toast("Не удалось импортировать: " + error.getMessage());
        }
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
        boolean retrying = running && status.startsWith("Переподключение");
        boolean failed = !connected && !starting && !stopping && !retrying && !"Отключено".equals(status);
        int tone = failed ? CORAL : connected || starting || retrying ? MINT : MUTED;
        chip.setText(connected ? "В СЕТИ" : starting ? "СОЕДИНЕНИЕ" : retrying ? "ПОВТОР" :
            stopping ? "ОТКЛЮЧЕНИЕ" : failed ? "СБОЙ" : "НЕ АКТИВЕН");
        chip.setTextColor(tone);
        chip.setBackground(surface(connected ? 0xff17342a : SURFACE, 20, BORDER));

        headline.setText(connected ? "VPN подключён" : starting ? "Подключаемся…" :
            stopping ? "Отключаемся…" : retrying ? "Восстанавливаем связь…" :
            failed ? "Соединение потеряно" : "VPN выключен");
        details.setText(connected ? "IPv4-трафик проходит через туннель." :
            starting ? "Устанавливаем защищённое соединение." :
            stopping ? "Завершаем текущий сеанс." : retrying || failed ? status :
            "Нажмите на стрелку, чтобы включить туннель.");

        hero.setState(connected, starting || stopping || running);
    }

    private void renderProfile() {
        ProfileStore.Profile profile = ProfileStore.active(this);
        if (profile == null) {
            activeProfileName.setText("Сервер не добавлен");
            activeProfileAddress.setText("Добавьте профиль для подключения");
            activeProfileAction.setText("Настроить  ›");
            return;
        }
        activeProfileName.setText(profile.name);
        activeProfileAddress.setText(profile.host + ":" + profile.port);
        activeProfileAction.setText("Сменить  ›");
    }

    private void openSettings() {
        startActivityForResult(new Intent(this, SettingsActivity.class), REQUEST_SETTINGS);
    }

    private void toggleConnection() {
        if (BladeVpn.running) {
            startService(new Intent(this, BladeVpn.class).setAction(BladeVpn.DISCONNECT));
            return;
        }
        if (ProfileStore.active(this) == null) {
            toast("Сначала добавьте профиль сервера в настройках");
            startActivityForResult(new Intent(this, SettingsActivity.class), REQUEST_SETTINGS);
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

    private void addFlexibleSpace(LinearLayout parent) {
        parent.addView(new View(this), new LinearLayout.LayoutParams(-1, 0, 1f));
    }

    private ImageView iconButton(int drawable, int box, int tint, int background, int radius) {
        ImageView icon = new ImageView(this);
        icon.setImageResource(drawable);
        icon.setColorFilter(tint);
        icon.setScaleType(ImageView.ScaleType.FIT_CENTER);
        icon.setBackground(surface(background, radius, 0));
        int padding = dp((box - 20) / 2f);
        icon.setPadding(padding, padding, padding, padding);
        icon.setClickable(true);
        icon.setForeground(new RippleDrawable(
            ColorStateList.valueOf(0x40ffffff), null,
            surface(background, radius, 0)));
        return icon;
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

    /**
     * Rings, glow, waves and particles around the central arrow. The waves and
     * particles stay subtle on purpose: three expanding rings, nine slow dots.
     */
    private final class HeroView extends View {
        private static final int PARTICLES = 9;
        private static final long WAVE_MS = 1900;
        private static final int WAVES = 3;

        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint glow = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final float[] angle = new float[PARTICLES];
        private final float[] orbitRadius = new float[PARTICLES];
        private final float[] dotRadius = new float[PARTICLES];
        private final float[] angularSpeed = new float[PARTICLES];
        private boolean connected;
        private boolean animating;
        private boolean animationVisible = true;
        private long stateSince;
        private RadialGradient connectedGlow;
        private RadialGradient connectingGlow;

        HeroView(MainActivity activity) {
            super(activity);
            setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
            java.util.Random random = new java.util.Random(7);
            for (int i = 0; i < PARTICLES; i++) {
                angle[i] = random.nextFloat() * 360f;
                orbitRadius[i] = 62f + random.nextFloat() * 48f;
                dotRadius[i] = 1.4f + random.nextFloat() * 1.2f;
                angularSpeed[i] = (2.5f + random.nextFloat() * 4f) * (random.nextBoolean() ? 1f : -1f);
            }
        }

        void setState(boolean nowConnected, boolean nowAnimating) {
            if (nowConnected != connected || nowAnimating != animating) {
                connected = nowConnected;
                animating = nowConnected || nowAnimating;
                stateSince = SystemClock.elapsedRealtime();
                invalidate();
            }
            if (animating && animationVisible) postInvalidateOnAnimation();
        }

        @Override protected void onSizeChanged(int width, int height, int oldWidth, int oldHeight) {
            super.onSizeChanged(width, height, oldWidth, oldHeight);
            float[] stops = {0f, 0.55f, 1f};
            connectedGlow = new RadialGradient(width / 2f, height / 2f, dp(86),
                new int[]{0x66b7f4cd, 0x267fe4b0, 0x007fe4b0}, stops, android.graphics.Shader.TileMode.CLAMP);
            connectingGlow = new RadialGradient(width / 2f, height / 2f, dp(86),
                new int[]{0x5576d9e8, 0x2276d9e8, 0x0076d9e8}, stops, android.graphics.Shader.TileMode.CLAMP);
        }

        void setAnimationVisible(boolean visible) {
            animationVisible = visible;
            if (visible && animating) postInvalidateOnAnimation();
        }

        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            float cx = getWidth() / 2f;
            float cy = getHeight() / 2f;
            long t = SystemClock.elapsedRealtime();
            float arrowRadius = dp(41);

            // The glow signals both connection setup and an active tunnel.
            if (animating) {
                float pulse = 0.78f + 0.22f * (float) Math.sin((t - stateSince) / 290.0);
                glow.setShader(connected ? connectedGlow : connectingGlow);
                glow.setAlpha((int) (255 * pulse));
                canvas.drawCircle(cx, cy, dp(86), glow);
            }

            // Static guide rings (idle and active).
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(dp(1));
            paint.setColor(connected ? 0x427fe4b0 : 0x315c91aa);
            for (int radius = 57; radius <= 125; radius += 17) {
                canvas.drawCircle(cx, cy, dp(radius), paint);
            }

            // Expanding rings show progress before the server confirms the connection.
            if (animating) {
                float maxRadius = Math.min(getWidth(), getHeight()) / 2f - dp(6);
                for (int i = 0; i < WAVES; i++) {
                    float phase = ((t - stateSince) % WAVE_MS) / (float) WAVE_MS;
                    phase = (phase + i / (float) WAVES) % 1f;
                    float radius = arrowRadius + phase * (maxRadius - arrowRadius);
                    paint.setStrokeWidth(dp(2) * (1f - phase * 0.7f));
                    paint.setColor(connected ? 0x667fe4b0 : 0x6676d9e8);
                    paint.setAlpha((int) ((connected ? 0x70 : 0x58) * (1f - phase)));
                    canvas.drawCircle(cx, cy, radius, paint);
                }
                paint.setStrokeWidth(dp(1));
            }

            // Counter-rotating arcs make connection setup visible at a glance.
            if (animating && !connected) {
                float rotation = (t - stateSince) % 1250 / 1250f * 360f;
                paint.setStyle(Paint.Style.STROKE);
                paint.setStrokeWidth(dp(2.5f));
                paint.setColor(0x9976d9e8);
                canvas.drawArc(cx - dp(48), cy - dp(48), cx + dp(48), cy + dp(48),
                    rotation, 115f, false, paint);
                paint.setStrokeWidth(dp(1.5f));
                paint.setColor(0x667fe4b0);
                canvas.drawArc(cx - dp(59), cy - dp(59), cx + dp(59), cy + dp(59),
                    -rotation * 0.72f, 72f, false, paint);
            }

            // Orbiting particles mark connection setup and the active tunnel.
            if (animating) {
                float fade = Math.min(1f, (t - stateSince) / 450f);
                float maxRadius = Math.min(getWidth(), getHeight()) / 2f - dp(8);
                for (int i = 0; i < PARTICLES; i++) {
                    float a = (float) Math.toRadians(angle[i] + (t / 1000f) * angularSpeed[i]);
                    float r = Math.min(dp(orbitRadius[i]), maxRadius)
                        + dp(5f) * (float) Math.sin((t / 900f) + i);
                    float x = cx + (float) Math.cos(a) * r;
                    float y = cy + (float) Math.sin(a) * r;
                    paint.setStyle(Paint.Style.FILL);
                    paint.setColor(!connected ? 0xffa9ebf2 :
                        i % 3 == 0 ? 0xffeafcf2 : 0xffb7f4cd);
                    paint.setAlpha((int) (fade * (i % 3 == 0 ? 150 : 105)));
                    canvas.drawCircle(x, y, dp(dotRadius[i]), paint);
                }
            }

            if (animating && animationVisible) postInvalidateOnAnimation();
        }
    }

    @Override protected void onStart() {
        super.onStart();
        if (hero != null) hero.setAnimationVisible(true);
        refresh.run();
    }

    @Override protected void onStop() {
        handler.removeCallbacks(refresh);
        if (hero != null) hero.setAnimationVisible(false);
        super.onStop();
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == REQUEST_VPN && result == RESULT_OK) connect();
        if (request == REQUEST_SETTINGS) {
            renderProfile();
            renderStatus();
        }
    }

    private void connect() {
        ProfileStore.Profile profile = ProfileStore.active(this);
        if (profile == null) return;
        Intent launch = new Intent(this, BladeVpn.class)
            .putExtra("name", profile.name)
            .putExtra("host", profile.host)
            .putExtra("port", profile.port)
            .putExtra("sni", profile.serverName)
            .putExtra("marker", profile.marker)
            .putExtra("secretBase64",
                android.util.Base64.encodeToString(profile.secret, android.util.Base64.NO_WRAP));
        if (profile.caPem != null && !profile.caPem.isEmpty()) {
            launch.putExtra("caPem", profile.caPem);
        }
        startForegroundService(launch);
    }
}
