package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.os.Build;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.util.List;

/** Manage all existing profiles, import and edit flows without migrating stored data. */
public final class ProfilesActivity extends Activity {
    private static final int BACKGROUND = 0xff0b121a;
    private static final int SURFACE = 0xff111e2a;
    private static final int BORDER = 0xff263746;
    private static final int WHITE = 0xffeff9f6;
    private static final int MUTED = 0xffacbfcc;
    private static final int MINT = 0xffb7f4cd;
    private static final int CORAL = 0xffffb4a8;
    private static final int REQUEST_EDIT = 2;
    private static final Typeface MEDIUM = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private static final Typeface BOLD = Typeface.create("sans-serif", Typeface.BOLD);

    private LinearLayout profileList;
    private TextView activeName;
    private TextView activeEndpoint;
    private AlertDialog dialog;

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
        content.setPadding(dp(24), dp(24), dp(24), dp(24));
        scroll.addView(content);
        setContentView(scroll);

        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        ImageView back = iconButton(R.drawable.ic_arrow_right, 40, MINT, SURFACE, 12);
        back.setRotation(180);
        back.setContentDescription("Назад");
        back.setOnClickListener(view -> finish());
        header.addView(back, new LinearLayout.LayoutParams(dp(40), dp(40)));
        TextView title = label("Профили", 20, WHITE, BOLD);
        LinearLayout.LayoutParams titleParams = new LinearLayout.LayoutParams(0, -2, 1f);
        titleParams.setMarginStart(dp(12));
        header.addView(title, titleParams);
        add(content, header, 0);

        TextView intro = label(
            "Добавьте сервер по ссылке от администратора или заполните данные вручную.",
            13, MUTED, null);
        intro.setLineSpacing(dp(3), 1);
        add(content, intro, 14);

        // Active profile card.
        LinearLayout endpoint = new LinearLayout(this);
        endpoint.setGravity(Gravity.CENTER_VERTICAL);
        endpoint.setBackground(surface(SURFACE, 20, BORDER));
        endpoint.setPadding(dp(16), dp(12), dp(16), dp(12));
        endpoint.setClickable(true);
        endpoint.setOnClickListener(view -> showProfilesDialog());
        LinearLayout server = column();
        server.addView(label("ТЕКУЩИЙ СЕРВЕР", 10, MUTED, MEDIUM));
        activeName = label("—", 16, WHITE, BOLD);
        add(server, activeName, 3);
        activeEndpoint = label("Нажмите, чтобы выбрать сервер", 12, MUTED, null);
        add(server, activeEndpoint, 2);
        endpoint.addView(server, new LinearLayout.LayoutParams(0, -2, 1));
        TextView manage = label("Выбрать ▸", 13, MINT, MEDIUM);
        manage.setPadding(dp(10), dp(6), dp(10), dp(6));
        manage.setBackground(surface(BACKGROUND, 12, BORDER));
        endpoint.addView(manage);
        add(content, endpoint, 12);

        add(content, caption("ДОБАВИТЬ СЕРВЕР"), 22);
        LinearLayout actions = new LinearLayout(this);
        Button paste = pill("Вставить ссылку");
        paste.setOnClickListener(view -> importFromClipboard());
        actions.addView(paste, new LinearLayout.LayoutParams(0, -2, 1f));
        LinearLayout.LayoutParams gap = new LinearLayout.LayoutParams(dp(10), 0);
        actions.addView(new View(this), gap);
        Button create = pill("Добавить вручную");
        create.setOnClickListener(view -> openEditor(null));
        actions.addView(create, new LinearLayout.LayoutParams(0, -2, 1f));
        add(content, actions, 8);

        TextView importHint = label(
            "Если ссылка открыта в браузере или чате, нажмите на неё — Blade добавит сервер сам.",
            12, MUTED, null);
        importHint.setLineSpacing(dp(3), 1);
        add(content, importHint, 8);

        add(content, caption("СОХРАНЁННЫЕ СЕРВЕРЫ"), 22);
        profileList = column();
        add(content, profileList, 4);

        TextView hint = label(
            "Ссылку sslim:// выдаёт администратор сервера вместе с личным секретом. "
                + "Кликните по ней в браузере или чате — Blade импортирует профиль автоматически.",
            12, MUTED, null);
        hint.setLineSpacing(dp(3), 1);
        add(content, hint, 18);
    }

    @Override protected void onResume() {
        super.onResume();
        render();
    }

    private void render() {
        ProfileStore.Profile active = ProfileStore.active(this);
        if (active == null) {
            activeName.setText("нет профиля");
            activeEndpoint.setText("Создайте или вставьте sslim:// ссылку");
        } else {
            activeName.setText(active.name);
            activeEndpoint.setText(active.host + ":" + active.port + " · SNI " + active.serverName);
        }
        profileList.removeAllViews();
        List<ProfileStore.Profile> profiles = ProfileStore.all(this);
        if (profiles.isEmpty()) {
            TextView empty = label("Пока пусто — добавьте первый профиль.", 13, MUTED, null);
            profileList.addView(empty, new LinearLayout.LayoutParams(-1, -2));
            return;
        }
        for (final ProfileStore.Profile profile : profiles) {
            boolean isActive = active != null && active.id.equals(profile.id);
            profileList.addView(profileRow(profile, isActive),
                new LinearLayout.LayoutParams(-1, -2) {{ topMargin = dp(8); }});
        }
    }

    private LinearLayout profileRow(ProfileStore.Profile profile, boolean isActive) {
        LinearLayout row = new LinearLayout(this);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setPadding(dp(16), dp(12), dp(16), dp(12));
        row.setBackground(surface(isActive ? 0xff17342a : SURFACE, 16, BORDER));
        row.setClickable(true);
        row.setOnClickListener(view -> {
            ProfileStore.setActive(this, profile.id);
            render();
            toast("Активный профиль: " + profile.name);
        });
        LinearLayout text = column();
        text.addView(label(profile.name, 15, WHITE, BOLD));
        add(text, label(profile.host + ":" + profile.port + " · " + profile.serverName, 12, MUTED, null), 2);
        row.addView(text, new LinearLayout.LayoutParams(0, -2, 1f));
        TextView badge = isActive ? label("АКТИВЕН", 10, MINT, MEDIUM) : label("", 10, MINT, MEDIUM);
        if (isActive) badge.setPadding(dp(8), dp(4), dp(8), dp(4));
        row.addView(badge);
        TextView edit = label("✎", 16, MINT, MEDIUM);
        edit.setPadding(dp(14), dp(8), dp(4), dp(8));
        edit.setClickable(true);
        edit.setOnClickListener(view -> openEditor(profile.id));
        row.addView(edit);
        TextView remove = label("✕", 16, CORAL, MEDIUM);
        remove.setPadding(dp(10), dp(8), dp(4), dp(8));
        remove.setClickable(true);
        remove.setOnClickListener(view -> confirmDelete(profile));
        row.addView(remove);
        return row;
    }

    private void showProfilesDialog() {
        List<ProfileStore.Profile> profiles = ProfileStore.all(this);
        if (profiles.isEmpty()) {
            openEditor(null);
            return;
        }
        LinearLayout list = column();
        for (final ProfileStore.Profile profile : profiles) {
            ProfileStore.Profile active = ProfileStore.active(this);
            LinearLayout row = new LinearLayout(this);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(dp(16), dp(12), dp(16), dp(12));
            row.setBackground(surface(active != null && active.id.equals(profile.id)
                ? 0xff17342a : SURFACE, 16, BORDER));
            row.setClickable(true);
            LinearLayout text = column();
            text.addView(label(profile.name, 15, WHITE, BOLD));
            add(text, label(profile.host + ":" + profile.port + " · " + profile.serverName, 12, MUTED, null), 2);
            row.addView(text, new LinearLayout.LayoutParams(0, -2, 1f));
            row.setOnClickListener(view -> {
                ProfileStore.setActive(this, profile.id);
                if (dialog != null) dialog.dismiss();
                render();
            });
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
            params.topMargin = dp(8);
            list.addView(row, params);
        }
        ScrollView listScroll = new ScrollView(this);
        listScroll.addView(list);
        listScroll.setPadding(dp(16), dp(16), dp(16), dp(8));
        dialog = new AlertDialog.Builder(this)
            .setTitle("Выбрать профиль")
            .setView(listScroll)
            .setNegativeButton("Закрыть", null)
            .setPositiveButton("Новый профиль", (d, which) -> openEditor(null))
            .show();
    }

    private void openEditor(String id) {
        Intent editor = new Intent(this, ProfileEditorActivity.class);
        editor.putExtra("id", id);
        startActivityForResult(editor, REQUEST_EDIT);
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
            render();
            toast("Профиль «" + imported.name + "» добавлен");
        } catch (Exception error) {
            toast("Не удалось импортировать: " + error.getMessage());
        }
    }

    private void confirmDelete(ProfileStore.Profile profile) {
        new AlertDialog.Builder(this)
            .setTitle("Удалить «" + profile.name + "»?")
            .setPositiveButton("Удалить", (dialogInterface, which) -> {
                ProfileStore.delete(this, profile.id);
                render();
                if (dialog != null) dialog.dismiss();
            })
            .setNegativeButton("Отмена", null)
            .show();
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == REQUEST_EDIT) render();
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }

    private TextView caption(String text) {
        return label(text, 11, MUTED, MEDIUM);
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
