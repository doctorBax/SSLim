package ru.doctorbax.blade;

import android.animation.ValueAnimator;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.animation.DecelerateInterpolator;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;

/**
 * Shared motion system for every Blade screen.
 * Decorative views never receive touches. System-wide reduced/disabled motion
 * is respected; network/VPN actions always run independently of animations.
 */
final class BladeMotion {
    private static final long ENTER_MS = 280;
    private static final DecelerateInterpolator EASE = new DecelerateInterpolator(1.5f);

    private BladeMotion() {}

    static boolean allowed() {
        return Build.VERSION.SDK_INT < 26 || ValueAnimator.areAnimatorsEnabled();
    }

    static int dp(Context context, float value) {
        return Math.round(value * context.getResources().getDisplayMetrics().density);
    }

    /** Mount the same restrained animated background on every screen. */
    static void mount(Activity activity, ScrollView scroll) {
        FrameLayout scene = new FrameLayout(activity);
        scene.setBackgroundColor(BladeAppearance.background(activity));
        BladeAmbientView backdrop = new BladeAmbientView(activity);
        backdrop.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        backdrop.setClickable(false);
        scene.addView(backdrop, new FrameLayout.LayoutParams(-1, dp(activity, 340), Gravity.TOP));
        scroll.setBackgroundColor(Color.TRANSPARENT);
        scene.addView(scroll, new FrameLayout.LayoutParams(-1, -1));
        activity.setContentView(scene);
    }

    /** Short staggered entry for sections, with no animations on text editing. */
    static void enter(LinearLayout content) {
        if (!allowed()) return;
        content.post(() -> {
            if (!content.isAttachedToWindow()) return;
            int count = Math.min(content.getChildCount(), 18);
            for (int i = 0; i < count; i++) {
                View child = content.getChildAt(i);
                if (child.getVisibility() != View.VISIBLE) continue;
                child.animate().cancel();
                child.setAlpha(0f);
                child.setTranslationY(dp(content.getContext(), 9));
                child.animate()
                    .alpha(1f).translationY(0f)
                    .setStartDelay(Math.min(220, i * 28L))
                    .setDuration(ENTER_MS)
                    .setInterpolator(EASE)
                    .start();
            }
        });
    }

    /** Simple status change: only when the underlying connection state changes. */
    static void state(View label) {
        if (!allowed() || !label.isAttachedToWindow()) return;
        label.animate().cancel();
        label.setAlpha(.55f);
        label.setTranslationY(dp(label.getContext(), 5));
        label.animate().alpha(1).translationY(0)
            .setDuration(220).setInterpolator(EASE).start();
    }

    static void selected(View row) {
        if (!allowed()) return;
        row.animate().cancel();
        row.setAlpha(.76f);
        row.animate().alpha(1f).setDuration(220).setInterpolator(EASE).start();
    }

    /**
     * Passive touch feedback: return false so clicks, switches and accessibility
     * services retain their normal Android behavior.
     */
    static void touch(View view) {
        view.setOnTouchListener((v, event) -> {
            if (!allowed() || !v.isEnabled()) return false;
            switch (event.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    v.animate().cancel();
                    v.animate().scaleX(.983f).scaleY(.983f)
                        .setDuration(85).start();
                    break;
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL:
                    v.animate().cancel();
                    v.animate().scaleX(1f).scaleY(1f)
                        .setDuration(180).setInterpolator(EASE).start();
                    break;
                default: break;
            }
            return false;
        });
    }

    /** Called on interactions only, never on layout or settings refresh. */
    static void bind(View view, Runnable action) {
        touch(view);
        view.setOnClickListener(v -> {
            BladeAppearance.haptic(v);
            action.run();
        });
    }

    static void transition(Activity from) {
        if (allowed()) from.overridePendingTransition(
            android.R.anim.fade_in, android.R.anim.fade_out);
    }

    static void navigate(Activity from, Intent intent) {
        from.startActivity(intent);
        transition(from);
    }

    static void finish(Activity activity) {
        activity.finish();
        if (allowed()) activity.overridePendingTransition(android.R.anim.fade_in, android.R.anim.fade_out);
    }

    /** The dialog theme follows Blade light/dark, not the fixed manifest theme. */
    static AlertDialog.Builder builder(Activity activity) {
        return new AlertDialog.Builder(activity, BladeAppearance.isDark(activity)
            ? android.R.style.Theme_Material_Dialog_Alert
            : android.R.style.Theme_Material_Light_Dialog_Alert);
    }

    static AlertDialog show(AlertDialog dialog) {
        dialog.show();
        Window window = dialog.getWindow();
        if (window == null) return dialog;
        GradientDrawable panel = new GradientDrawable();
        panel.setColor(BladeAppearance.surface(dialog.getContext()));
        panel.setCornerRadius(dp(dialog.getContext(), 24));
        panel.setStroke(dp(dialog.getContext(), 1), BladeAppearance.border(dialog.getContext()));
        window.setBackgroundDrawable(panel);
        int accent = BladeAppearance.accent(dialog.getContext());
        if (dialog.getButton(AlertDialog.BUTTON_POSITIVE) != null)
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setTextColor(accent);
        if (dialog.getButton(AlertDialog.BUTTON_NEGATIVE) != null)
            dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setTextColor(accent);
        if (dialog.getButton(AlertDialog.BUTTON_NEUTRAL) != null)
            dialog.getButton(AlertDialog.BUTTON_NEUTRAL).setTextColor(accent);
        if (allowed()) {
            View decor = window.getDecorView();
            decor.setAlpha(0f);
            decor.setScaleX(.97f);
            decor.setScaleY(.97f);
            decor.animate().alpha(1f).scaleX(1f).scaleY(1f)
                .setDuration(210).setInterpolator(EASE).start();
        }
        return dialog;
    }

    static AlertDialog show(AlertDialog.Builder builder) {
        return show(builder.create());
    }
}
