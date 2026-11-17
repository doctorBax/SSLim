package ru.doctorbax.blade;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.LinearGradient;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RadialGradient;
import android.graphics.Shader;
import android.os.SystemClock;
import android.view.View;

/**
 * Background-only connection scene. No click handlers or central button:
 * MainActivity owns the entire clickable panel and keeps the arrow unchanged.
 * The same View is used in AppearanceActivity's interactive preview.
 */
final class BladeHeroView extends View {
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint glow = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final float[] phase = new float[13];
    private final float[] radius = new float[13];
    private final float[] speed = new float[13];
    private long started = SystemClock.elapsedRealtime();
    private boolean connected;
    private boolean animating;
    private boolean visible = true;
    private float activity;
    private long lastUp;
    private long lastDown;
    private long lastTrafficSample;
    private RadialGradient gradient;
    private int gradientColor;

    BladeHeroView(Context context) {
        super(context);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        java.util.Random rand = new java.util.Random(7);
        for (int i = 0; i < phase.length; i++) {
            phase[i] = rand.nextFloat() * 360f;
            radius[i] = 58f + rand.nextFloat() * 54f;
            speed[i] = (2.5f + rand.nextFloat() * 5f) * (i % 2 == 0 ? 1f : -1f);
        }
    }

    void setState(boolean nowConnected, boolean nowAnimating) {
        if (connected != nowConnected || animating != nowAnimating) {
            connected = nowConnected;
            animating = nowAnimating || nowConnected;
            started = SystemClock.elapsedRealtime();
            invalidate();
        }
        if (animating && visible) postInvalidateOnAnimation();
    }

    void setAnimationVisible(boolean value) {
        visible = value;
        if (visible && animating) postInvalidateOnAnimation();
    }

    void setTraffic(long uploaded, long downloaded) {
        long now = SystemClock.elapsedRealtime();
        if (lastTrafficSample > 0 && now > lastTrafficSample) {
            long delta = Math.max(0L, uploaded - lastUp) + Math.max(0L, downloaded - lastDown);
            float instant = Math.min(1f, delta * 1000f / (now - lastTrafficSample) / 2_000_000f);
            activity = 0.72f * activity + 0.28f * instant;
        }
        lastTrafficSample = now;
        lastUp = uploaded;
        lastDown = downloaded;
    }

    private int dp(float x) {
        return (int) (x * getResources().getDisplayMetrics().density + 0.5f);
    }

    private static int alpha(int color, int a) {
        return (color & 0x00ffffff) | (Math.max(0, Math.min(255, a)) << 24);
    }

    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        int style = BladeAppearance.animation(getContext());
        int accent = BladeAppearance.accent(getContext());
        float t = (SystemClock.elapsedRealtime() - started) / 1000f;
        float x = getWidth() * .5f;
        float y = getHeight() * .5f;
        paint.setShader(null);
        paint.setStrokeCap(Paint.Cap.ROUND);
        paint.setStrokeJoin(Paint.Join.ROUND);
        switch (style) {
            case 1: aurora(canvas, x, y, t, accent); break;
            case 2: dataStream(canvas, x, y, t, accent); break;
            case 3: spectrum(canvas, x, y, t, accent); break;
            case 4: crystal(canvas, x, y, t, accent); break;
            default: orbit(canvas, x, y, t, accent);
        }
        paint.setShader(null);
        if (animating && visible) postInvalidateOnAnimation();
    }

    private void orbit(Canvas canvas, float x, float y, float t, int accent) {
        int guide = alpha(accent, connected ? 65 : 40);
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(dp(1));
        paint.setColor(guide);
        for (int r = 57; r <= 125; r += 17) {
            canvas.drawCircle(x, y, dp(r), paint);
        }
        if (animating) {
            float range = Math.min(getWidth(), getHeight()) / 2 - dp(6);
            if (gradient == null || gradientColor != accent) {
                gradientColor = accent;
                gradient = new RadialGradient(x, y, dp(86), new int[]{
                    alpha(accent, 84), alpha(accent, 29), alpha(accent, 0)},
                    null, Shader.TileMode.CLAMP);
            }
            glow.setShader(gradient);
            glow.setAlpha((int)(220 * (0.75f + .25f * Math.sin(t * 2))));
            glow.setStyle(Paint.Style.FILL);
            canvas.drawCircle(x, y, dp(86), glow);
            glow.setShader(null);
            paint.setStyle(Paint.Style.STROKE);
            for (int i = 0; i < 3; i++) {
                float p = ((t / 1.9f) + i / 3f) % 1;
                paint.setStrokeWidth(dp(2) * (1f - p * .65f));
                paint.setColor(alpha(accent, (int)(118 * (1 - p))));
                canvas.drawCircle(x, y, dp(41) + p * (range - dp(41)), paint);
            }
            if (!connected) {
                float sweep = (t * 260) % 360;
                paint.setStrokeWidth(dp(2.6f));
                paint.setColor(alpha(accent, 155));
                canvas.drawArc(x - dp(49), y - dp(49), x + dp(49), y + dp(49),
                    sweep, 110, false, paint);
            }
            paint.setStyle(Paint.Style.FILL);
            for (int i = 0; i < phase.length; i++) {
                float angle = (float)Math.toRadians(phase[i] + t * speed[i]);
                float r = Math.min(dp(radius[i]), range);
                paint.setColor(alpha(accent, i % 3 == 0 ? 160 : 102));
                canvas.drawCircle(x + (float)Math.cos(angle) * r,
                    y + (float)Math.sin(angle) * r, dp(i % 3 == 0 ? 2.3f : 1.6f), paint);
            }
        }
    }

    /** Long flowing ribbons, not circular particles. */
    private void aurora(Canvas canvas, float cx, float cy, float t, int accent) {
        float w = getWidth(), h = getHeight();
        int secondary = BladeAppearance.isDark(getContext()) ? 0xffa99ef8 : 0xff746cb7;
        for (int i = 0; i < 5; i++) {
            float p = i / 4f;
            float motion = animating ? t * (.28f + .04f * i) : 0f;
            Path ribbon = new Path();
            float top = h * (.19f + .11f * i);
            float thickness = h * (.085f + .025f * (4-i));
            ribbon.moveTo(-10, top);
            for (int j = 0; j <= 30; j++) {
                float px = w * j / 30f;
                float wave = (float)Math.sin(px / Math.max(w,1) * 6.5f + motion + p * 2.7f);
                float wave2 = (float)Math.sin(px / Math.max(w,1) * 12f - motion*.3f + i);
                float py = top + (wave*.7f + wave2*.3f) * h * .10f;
                ribbon.lineTo(px, py);
            }
            for (int j = 30; j >= 0; j--) {
                float px = w * j / 30f;
                float wave = (float)Math.sin(px / Math.max(w,1) * 6.5f + motion + p * 2.7f);
                float wave2 = (float)Math.sin(px / Math.max(w,1) * 12f - motion*.3f + i);
                float py = top + thickness + (wave*.7f + wave2*.3f) * h * .10f;
                ribbon.lineTo(px, py);
            }
            ribbon.close();
            paint.setStyle(Paint.Style.FILL);
            paint.setShader(new LinearGradient(0, top-25, w, top+thickness+30,
                alpha(i%2==0?accent:secondary, connected?52:28),
                alpha(i%2==0?secondary:accent, connected?94:52), Shader.TileMode.CLAMP));
            canvas.drawPath(ribbon, paint);
            paint.setShader(null);
        }
    }

    /** Moving network lanes and packets; direction alternates per lane. */
    private void dataStream(Canvas canvas, float cx, float cy, float t, int accent) {
        float w=getWidth(), h=getHeight();
        float boost = animating ? .4f + activity * 2.3f : 0f;
        for (int i=0; i<9; i++) {
            float y = h*(i+1)/10f;
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(dp(1));
            paint.setColor(alpha(accent, 33));
            canvas.drawLine(dp(10), y, w-dp(10), y, paint);
            paint.setStyle(Paint.Style.FILL);
            int count = 2 + i%3;
            for(int j=0;j<count;j++) {
                float segment = w+dp(55);
                float offset=(i*41f+j*(segment/count));
                float pos = (offset + (i%2==0 ? 1 : -1)*t*(dp(17)+dp(22)*boost)) % segment;
                if(pos<0)pos+=segment;
                float left=pos-dp(40);
                paint.setColor(alpha(accent, 55+i%4*20));
                canvas.drawRoundRect(left,y-dp(2.3f),left+dp(23+i%2*13),
                    y+dp(2.3f),dp(2),dp(2),paint);
            }
        }
        // Four static vertical guides make the panel read like a data circuit.
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(dp(.65f));
        paint.setColor(alpha(accent, 28));
        for (int i=1;i<=4;i++) canvas.drawLine(w*i/5f, dp(10),w*i/5f,h-dp(10),paint);
    }

    /** Equalizer responds to actual traffic; an idle animation stays restrained. */
    private void spectrum(Canvas canvas, float cx, float cy, float t, int accent) {
        float w=getWidth(), h=getHeight();
        int bars=35;
        float gap = w / (bars+2f);
        for(int i=0;i<bars;i++) {
            float x=gap*(i+1.5f);
            float envelope = (float)Math.pow(Math.max(0f,
                1-Math.abs(i-(bars-1)*.5f)/(bars*.62f)), 1.2);
            float pulse = animating ? (float)Math.abs(Math.sin(t*1.8f + i*.42f)) : .24f;
            float height = dp(5) + h*(.10f + .38f*(.12f + pulse*.62f + activity*.48f))*envelope;
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(alpha(accent, 48+(int)(120*envelope)));
            canvas.drawRoundRect(x-gap*.22f,cy-height*.5f,x+gap*.22f,cy+height*.5f,
                gap*.22f,gap*.22f,paint);
            if (i%4==0) {
                paint.setColor(alpha(accent, 30));
                canvas.drawCircle(x,cy-height*.5f-dp(7),dp(1.5f),paint);
            }
        }
    }

    /** A deforming low-poly tessellation with luminous nodes and facets. */
    private void crystal(Canvas canvas, float cx, float cy, float t, int accent) {
        int columns=8, rows=5;
        float w=getWidth(),h=getHeight();
        float[][] xx=new float[rows][columns], yy=new float[rows][columns];
        float drift = animating ? 1f : 0f;
        for(int y=0;y<rows;y++) {
            for(int x=0;x<columns;x++) {
                int id=y*columns+x;
                float jitter=(float)Math.sin(id*19.3f)*.18f;
                xx[y][x] = w*(x+jitter+.35f)/(columns-0.3f)
                    + drift*dp(4)*(float)Math.sin(t*.7f+id);
                yy[y][x] = h*(y+.35f+(float)Math.cos(id*6.6f)*.13f)/(rows-.25f)
                    + drift*dp(7)*(float)Math.cos(t*.4f+id*1.8f);
            }
        }
        for(int y=0;y<rows-1;y++) for(int x=0;x<columns-1;x++) {
            float[] px={xx[y][x],xx[y][x+1],xx[y+1][x],xx[y+1][x+1]};
            float[] py={yy[y][x],yy[y][x+1],yy[y+1][x],yy[y+1][x+1]};
            Path a=new Path();
            a.moveTo(px[0],py[0]);a.lineTo(px[1],py[1]);a.lineTo(px[2],py[2]);a.close();
            paint.setStyle(Paint.Style.FILL);
            paint.setColor(alpha(accent,20+(x+y)%4*9));
            canvas.drawPath(a,paint);
            Path b=new Path();
            b.moveTo(px[1],py[1]);b.lineTo(px[2],py[2]);b.lineTo(px[3],py[3]);b.close();
            paint.setColor(alpha(accent,15+(x+y+2)%4*10));
            canvas.drawPath(b,paint);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(dp(.7f));
            paint.setColor(alpha(accent,60));
            canvas.drawPath(a,paint); canvas.drawPath(b,paint);
        }
        paint.setStyle(Paint.Style.FILL);
        for(int y=0;y<rows;y++) for(int x=0;x<columns;x++){
            paint.setColor(alpha(accent,115));
            canvas.drawCircle(xx[y][x],yy[y][x],dp(1.7f),paint);
        }
    }
}
