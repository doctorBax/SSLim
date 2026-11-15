package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.os.Bundle;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.nio.charset.StandardCharsets;
import java.util.Base64;

/** Create or edit one server profile. Every field is user-editable. */
public final class ProfileEditorActivity extends Activity {
    private static final int BACKGROUND = 0xff0b121a;
    private static final int SURFACE = 0xff111e2a;
    private static final int BORDER = 0xff263746;
    private static final int WHITE = 0xffeff9f6;
    private static final int MUTED = 0xffacbfcc;
    private static final int MINT = 0xffb7f4cd;
    private static final int CORAL = 0xffffb4a8;
    private static final Typeface MEDIUM = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private static final Typeface BOLD = Typeface.create("sans-serif", Typeface.BOLD);

    private String editingId;
    private String editingMarker = "sync/1";
    private EditText link;
    private EditText name;
    private EditText host;
    private EditText port;
    private EditText sni;
    private EditText secret;
    private CheckBox customCa;
    private EditText caPem;

    @Override public void onCreate(Bundle savedState) {
        super.onCreate(savedState);
        if (Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
        getWindow().setStatusBarColor(BACKGROUND);
        getWindow().setNavigationBarColor(BACKGROUND);

        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(BACKGROUND);
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets bars = insets.getInsets(
                    WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
                return insets;
            });
        }
        LinearLayout content = column();
        content.setPadding(dp(22), dp(22), dp(22), dp(22));
        scroll.addView(content);
        setContentView(scroll);

        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = label("Профиль сервера", 20, WHITE, BOLD);
        header.addView(title, new LinearLayout.LayoutParams(0, -2, 1f));
        TextView cancel = label("Отмена", 14, MUTED, MEDIUM);
        cancel.setPadding(dp(8), dp(8), dp(8), dp(8));
        cancel.setOnClickListener(view -> finish());
        header.addView(cancel);
        add(content, header, 0);

        editingId = getIntent() == null ? null : getIntent().getStringExtra("id");
        ProfileStore.Profile existing = null;
        if (editingId != null) {
            for (ProfileStore.Profile profile : ProfileStore.all(this)) {
                if (profile.id.equals(editingId)) existing = profile;
            }
        }

        link = field("Вставьте ссылку sslim:// целиком — поля заполнятся сами", false);
        add(content, caption("БЫСТРЫЙ ИМПОРТ"), 18);
        add(content, link, 6);
        Button fill = pill("Заполнить из ссылки");
        fill.setOnClickListener(view -> fillFromLink());
        add(content, fill, 8);

        add(content, caption("СЕРВЕР"), 20);
        name = field("Название (например, «Личный VPN»)", true);
        add(content, name, 6);
        host = field("Адрес: домен или IP", false);
        add(content, host, 8);
        port = field("Порт (обычно 9443)", false);
        port.setInputType(InputType.TYPE_CLASS_NUMBER);
        add(content, port, 8);
        sni = field("TLS server name (обычно совпадает с доменом)", false);
        add(content, sni, 8);

        add(content, caption("СЕКРЕТ (base64, минимум 32 байта)"), 20);
        secret = field("Секрет из sslim://-ссылки или от администратора сервера", true);
        secret.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
        add(content, secret, 6);

        add(content, caption("СЕРТИФИКАТ СЕРВЕРА"), 20);
        customCa = new CheckBox(this);
        customCa.setText("Свой CA (PEM) вместо системных сертификатов");
        customCa.setTextColor(WHITE);
        customCa.setTextSize(13);
        customCa.setOnCheckedChangeListener((button, checked) -> caPem.setVisibility(checked ? View.VISIBLE : View.GONE));
        add(content, customCa, 4);
        caPem = field("PEM-сертификат сервера (-----BEGIN CERTIFICATE-----)", true);
        caPem.setVisibility(View.GONE);
        caPem.setMinLines(3);
        add(content, caPem, 6);

        Button save = new Button(this);
        save.setText(existing == null ? "Сохранить профиль" : "Сохранить изменения");
        save.setAllCaps(false);
        save.setTextSize(16);
        save.setTypeface(BOLD);
        save.setTextColor(BACKGROUND);
        save.setMinHeight(dp(54));
        save.setStateListAnimator(null);
        save.setBackground(new android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(0x280b121a),
            surface(MINT, 18, 0), null));
        save.setOnClickListener(view -> save());
        add(content, save, 24);

        if (existing != null) {
            Button delete = new Button(this);
            delete.setText("Удалить профиль");
            delete.setAllCaps(false);
            delete.setTextSize(14);
            delete.setTypeface(MEDIUM);
            delete.setTextColor(CORAL);
            delete.setMinHeight(dp(46));
            delete.setStateListAnimator(null);
            delete.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(0x30ffffff),
                surface(SURFACE, 16, BORDER), null));
            delete.setOnClickListener(view -> confirmDelete());
            add(content, delete, 10);
            editingMarker = existing.marker;
            fill(existing);
        }
    }

    private void fill(ProfileStore.Profile profile) {
        name.setText(profile.name);
        host.setText(profile.host);
        port.setText(String.valueOf(profile.port));
        sni.setText(profile.serverName);
        secret.setText(Base64.getEncoder().encodeToString(profile.secret));
        if (profile.caPem != null && !profile.caPem.isEmpty()) {
            customCa.setChecked(true);
            caPem.setText(profile.caPem);
        }
    }

    private void fillFromLink() {
        String source = link.getText().toString().replaceAll("\\s+", "");
        if (source.isEmpty()) {
            toast("Поле ссылки пустое");
            return;
        }
        try {
            ProfileStore.Profile parsed = ProfileStore.fromUri(source);
            fill(parsed);
            toast("Поля заполнены из ссылки");
        } catch (Exception error) {
            toast("Не удалось разобрать ссылку: " + error.getMessage());
        }
    }

    private void save() {
        try {
            String hostValue = host.getText().toString().trim();
            if (hostValue.isEmpty()) throw new IllegalArgumentException("Заполните адрес сервера");
            String portValue = port.getText().toString().trim();
            int portNumber = portValue.isEmpty() ? 9443 : Integer.parseInt(portValue);
            if (portNumber < 1 || portNumber > 65535) throw new IllegalArgumentException("Неверный порт");
            String sniValue = sni.getText().toString().trim();
            if (sniValue.isEmpty()) sniValue = hostValue;
            byte[] key = Base64.getMimeDecoder().decode(secret.getText().toString().trim());
            if (key.length < 32) throw new IllegalArgumentException("Секрет короче 32 байт");
            String nameValue = name.getText().toString().trim();
            if (nameValue.isEmpty()) nameValue = hostValue;
            String ca = customCa.isChecked() ? caPem.getText().toString().trim() : null;
            if (customCa.isChecked() && (ca == null || !ca.contains("BEGIN CERTIFICATE"))) {
                throw new IllegalArgumentException("PEM-сертификат не похож на сертификат");
            }
            ProfileStore.Profile profile = new ProfileStore.Profile(
                editingId == null ? ProfileStore.newId() : editingId,
                nameValue, hostValue, portNumber, sniValue, editingMarker, key, ca);
            ProfileStore.save(this, profile);
            setResult(RESULT_OK);
            finish();
        } catch (IllegalArgumentException error) {
            toast(error.getMessage());
        } catch (Exception error) {
            toast("Ошибка: " + error.getMessage());
        }
    }

    private void confirmDelete() {
        new AlertDialog.Builder(this)
            .setTitle("Удалить профиль?")
            .setPositiveButton("Удалить", (dialog, which) -> {
                ProfileStore.delete(this, editingId);
                setResult(RESULT_OK);
                finish();
            })
            .setNegativeButton("Отмена", null)
            .show();
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }

    private TextView caption(String text) {
        TextView view = label(text, 11, MUTED, MEDIUM);
        return view;
    }

    private EditText field(String hint, boolean multiline) {
        EditText view = new EditText(this);
        view.setHint(hint);
        view.setTextColor(WHITE);
        view.setHintTextColor(MUTED);
        view.setTextSize(14);
        view.setBackground(surface(SURFACE, 14, BORDER));
        view.setPadding(dp(14), dp(12), dp(14), dp(12));
        if (multiline) {
            view.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
            view.setMinLines(multiline ? 2 : 1);
        }
        return view;
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
        view.setBackground(new android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(0x30ffffff), surface(SURFACE, 16, BORDER), null));
        return view;
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
}
