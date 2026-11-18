package ru.doctorbax.blade;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.os.SystemClock;
import android.view.View;

/**
 * Deliberately low-contrast menu ambience, coordinated with the selected
 * connection animation. Geometry stays behind all readable content.
 * Max ~20 FPS, stopped automatically when a screen is not visible.
 */
final class BladeAmbientView extends View {
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private boolean visibleToUser;
    private final long start = SystemClock.elapsedRealtime();

    BladeAmbientView(Context context) {
        super(context);
        setLayerType(View.LAYER_TYPE_HARDWARE, null);
    }

    private float dp(float value) {
        return value * getResources().getDisplayMetrics().density;
    }

    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        visibleToUser = true;
        invalidate();
    }

    @Override protected void onDetachedFromWindow() {
        visibleToUser = false;
        super.onDetachedFromWindow();
    }

    @Override public void onVisibilityAggregated(boolean visible) {
        super.onVisibilityAggregated(visible);
        visibleToUser = visible;
        if (visible) invalidate();
    }

    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        float w = getWidth();
        float h = getHeight();
        if (w <= 0 || h <= 0) return;
        boolean animate = BladeMotion.allowed();
        float t = animate ? (SystemClock.elapsedRealtime() - start) * .001f : 0f;
        int accent = BladeAppearance.accent(getContext());
        int alpha = BladeAppearance.isDark(getContext()) ? 29 : 20;
        paint.setShader(null);
        paint.setStrokeCap(Paint.Cap.ROUND);
        paint.setStrokeWidth(dp(1));
        switch (BladeAppearance.animation(getContext())) {
            case 1: aurora(canvas,w,h,t,accent,alpha);break;
            case 2: dataStream(canvas,w,h,t,accent,alpha);break;
            case 3: spectrum(canvas,w,h,t,accent,alpha);break;
            case 4: crystal(canvas,w,h,t,accent,alpha);break;
            default: orbit(canvas,w,h,t,accent,alpha);
        }
        paint.setShader(null);
        if (animate && visibleToUser) postInvalidateDelayed(55);
    }

    private static int tinted(int color,int opacity) {
        return (color & 0x00ffffff) | (Math.max(0,Math.min(255,opacity)) << 24);
    }

    private void orbit(Canvas canvas,float w,float h,float t,int color,int opacity) {
        paint.setStyle(Paint.Style.STROKE);
        float x=w*.88f,y=h*.15f;
        for(int i=0;i<5;i++){
            float radius=dp(38+i*23)+(float)Math.sin(t*.4f+i)*dp(2);
            paint.setColor(tinted(color,opacity-(i*3)));
            canvas.drawCircle(x,y,radius,paint);
        }
        float angle=t*.27f;
        paint.setStyle(Paint.Style.FILL);
        paint.setColor(tinted(color,opacity*2));
        canvas.drawCircle(x+(float)Math.cos(angle)*dp(85),
            y+(float)Math.sin(angle)*dp(85),dp(3.2f),paint);
    }

    private void aurora(Canvas canvas,float w,float h,float t,int color,int opacity) {
        paint.setStyle(Paint.Style.STROKE);
        for(int i=0;i<5;i++){
            Path path=new Path();
            path.moveTo(-dp(12),h*(.09f+i*.08f));
            for(int step=0;step<=26;step++) {
                float x=w*step/26f;
                float y=h*(.09f+i*.08f)
                    + (float)Math.sin(x/w*5+t*.24f+i*.8f)*dp(18);
                path.lineTo(x,y);
            }
            paint.setStrokeWidth(dp(4+i%3));
            paint.setColor(tinted(color,opacity+i*3));
            canvas.drawPath(path,paint);
        }
    }

    private void dataStream(Canvas canvas,float w,float h,float t,int color,int opacity) {
        paint.setStrokeWidth(dp(1));
        for(int i=0;i<9;i++){
            float y=h*(.07f+i*.09f);
            paint.setColor(tinted(color,opacity/2));
            canvas.drawLine(0,y,w,y,paint);
            float shift=(t*(i%2==0?25:-22)+i*71)%Math.max(w,1);
            if(shift<0)shift+=w;
            paint.setStrokeWidth(dp(2+i%2));
            paint.setColor(tinted(color,opacity*2));
            canvas.drawLine(shift,y,Math.min(w,shift+dp(33+i*4)),y,paint);
            paint.setStrokeWidth(dp(1));
        }
    }

    private void spectrum(Canvas canvas,float w,float h,float t,int color,int opacity) {
        paint.setStyle(Paint.Style.FILL);
        float groupStart=w*.33f;
        for(int i=0;i<24;i++){
            float x=groupStart+i*(w*.62f/24f);
            float pulse=(float)Math.abs(Math.sin(t*.68f+i*.31f));
            float tall=dp(10)+dp(30)*pulse;
            paint.setColor(tinted(color,opacity+i%4*3));
            canvas.drawRoundRect(x,h*.22f-tall/2,x+dp(3),h*.22f+tall/2,
                dp(1.5f),dp(1.5f),paint);
        }
    }

    private void crystal(Canvas canvas,float w,float h,float t,int color,int opacity) {
        paint.setStyle(Paint.Style.STROKE);
        paint.setStrokeWidth(dp(.9f));
        float startX=w*.52f;
        for(int row=0;row<5;row++) {
            for(int col=0;col<5;col++) {
                float x=startX+col*w*.13f;
                float y=h*.05f+row*dp(55);
                float drift=(float)Math.sin(t*.32f+row*.7f+col)*dp(4);
                paint.setColor(tinted(color,opacity + (row+col)%3*4));
                Path triangle=new Path();
                triangle.moveTo(x,y+drift);
                triangle.lineTo(x+w*.13f,y+dp(20)-drift);
                triangle.lineTo(x+w*.04f,y+dp(55)+drift);
                triangle.close();
                canvas.drawPath(triangle,paint);
            }
        }
    }
}
