package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.os.Build;
import android.os.Bundle;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Switch;
import android.widget.TextView;

/** Live, interactive appearance configuration; changes persist without a restart. */
public final class AppearanceActivity extends Activity {
    private static final Typeface BOLD = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    private BladeHeroView preview;
    private boolean demoConnected = true;
    private String appliedStamp;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        demoConnected = state == null || state.getBoolean("demo", true);
        render();
    }

    @Override protected void onSaveInstanceState(Bundle out) {
        out.putBoolean("demo", demoConnected);
        super.onSaveInstanceState(out);
    }

    @Override protected void onStart() {
        super.onStart();
        if (preview != null) preview.setAnimationVisible(true);
    }

    @Override protected void onStop() {
        if (preview != null) preview.setAnimationVisible(false);
        super.onStop();
    }

    private int dp(float value) {
        return (int)(value * getResources().getDisplayMetrics().density + .5f);
    }

    private int bg() { return BladeAppearance.background(this); }
    private int surfaceColor() { return BladeAppearance.surface(this); }
    private int border() { return BladeAppearance.border(this); }
    private int primary() { return BladeAppearance.accent(this); }
    private int textColor() { return BladeAppearance.text(this); }
    private int muted() { return BladeAppearance.muted(this); }

    private GradientDrawable bgShape(int fill, int corner, int strokeColor) {
        GradientDrawable shape = new GradientDrawable();
        shape.setColor(fill);
        shape.setCornerRadius(dp(corner));
        if (strokeColor != 0) shape.setStroke(dp(1), strokeColor);
        return shape;
    }

    private LinearLayout column() {
        LinearLayout v = new LinearLayout(this);
        v.setOrientation(LinearLayout.VERTICAL);
        return v;
    }

    private void add(LinearLayout parent, View child, int top) {
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(-1, -2);
        lp.topMargin = dp(top);
        parent.addView(child, lp);
    }

    private TextView label(String content, int size, int color, boolean strong) {
        TextView result = new TextView(this);
        result.setText(content);
        result.setTextSize(size);
        result.setTextColor(color);
        if (strong) result.setTypeface(BOLD);
        return result;
    }

    private TextView section(String value) {
        return label(value, 11, primary(), true);
    }

    private void render() {
        if (preview != null) preview.setAnimationVisible(false);
        appliedStamp = BladeAppearance.stamp(this);
        BladeAppearance.applyWindow(this);
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(bg());
        scroll.setVerticalScrollBarEnabled(false);
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener((view,insets)->{
                android.graphics.Insets bars=insets.getInsets(
                    WindowInsets.Type.systemBars()|WindowInsets.Type.displayCutout());
                view.setPadding(bars.left,bars.top,bars.right,bars.bottom);
                return insets;
            });
        }
        LinearLayout root = column();
        root.setPadding(dp(24),dp(22),dp(24),dp(30));
        scroll.addView(root);
        setContentView(scroll);

        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView back = label("‹",36,primary(),false);
        back.setGravity(Gravity.CENTER_VERTICAL);
        back.setOnClickListener(view -> finish());
        back.setContentDescription("Назад");
        header.addView(back,new LinearLayout.LayoutParams(dp(42),dp(48)));
        header.addView(label("Оформление",21,textColor(),true));
        add(root,header,0);
        add(root,label("Персонализируйте Blade. Предпросмотр не подключается к VPN.",
            12,muted(),false),8);

        add(root,section("ИНТЕРАКТИВНЫЙ ПРЕДПРОСМОТР"),22);
        FrameLayout stage = new FrameLayout(this);
        stage.setBackground(new GradientDrawable(GradientDrawable.Orientation.TL_BR,
            new int[]{BladeAppearance.heroTop(this),BladeAppearance.heroBottom(this)}));
        stage.setClipToOutline(true);
        GradientDrawable stageBg = new GradientDrawable(GradientDrawable.Orientation.TL_BR,
            new int[]{BladeAppearance.heroTop(this), BladeAppearance.heroBottom(this)});
        stageBg.setCornerRadius(dp(23));
        stageBg.setStroke(dp(1),border());
        stage.setBackground(stageBg);
        stage.setClickable(true);
        preview = new BladeHeroView(this);
        preview.setState(demoConnected, demoConnected);
        stage.addView(preview,new FrameLayout.LayoutParams(-1,-1));
        ImageView arrow = new ImageView(this);
        arrow.setImageResource(R.drawable.ic_arrow_right);
        arrow.setColorFilter(BladeAppearance.onAccent(this));
        arrow.setPadding(dp(22),dp(22),dp(22),dp(22));
        arrow.setBackground(bgShape(primary(),BladeAppearance.square(this)?18:44,0));
        stage.addView(arrow,new FrameLayout.LayoutParams(dp(82),dp(82),Gravity.CENTER));
        stage.setContentDescription("Предпросмотр. Нажмите для имитации подключения.");
        stage.setOnClickListener(view -> {
            demoConnected=!demoConnected;
            if (BladeAppearance.haptics(this)) BladeAppearance.haptic(stage);
            preview.setState(demoConnected,demoConnected);
            invalidateOptionsMenu();
        });
        LinearLayout.LayoutParams stageLp = new LinearLayout.LayoutParams(-1,dp(212));
        stageLp.topMargin=dp(9);
        root.addView(stage,stageLp);
        TextView previewHint=label("Нажмите на панель, чтобы изменить состояние предпросмотра",
            12,muted(),false);
        previewHint.setGravity(Gravity.CENTER);
        add(root,previewHint,9);

        add(root,section("ТЕМА ПРИЛОЖЕНИЯ"),25);
        String[] names={"Системная","Тёмная","Светлая"};
        String[] values={BladeAppearance.SYSTEM,BladeAppearance.DARK,BladeAppearance.LIGHT};
        for(int i=0;i<names.length;i++) {
            final String value=values[i];
            add(root,choice(names[i],BladeAppearance.mode(this).equals(value),
                ()->{BladeAppearance.setMode(this,value);render();}),i==0?10:7);
        }

        add(root,section("АКЦЕНТНЫЙ ЦВЕТ"),25);
        String[] colors={BladeAppearance.MINT,BladeAppearance.BLUE,
            BladeAppearance.LAVENDER,BladeAppearance.AMBER,BladeAppearance.CUSTOM};
        String[] colorNames={"Стандартный · Blade Mint","Arctic Blue",
            "Soft Lavender","Warm Amber","Свой цвет · палитра HSV / HEX"};
        int[] swatch={0xffb7f4cd,0xff8cbdf0,0xffb5a2e8,0xffe9c68a,
            BladeAppearance.customColor(this)};
        for(int i=0;i<colors.length;i++){
            final String value=colors[i];
            LinearLayout row=new LinearLayout(this);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(dp(15),dp(11),dp(12),dp(11));
            row.setBackground(bgShape(surfaceColor(),15,border()));
            View dot=new View(this);
            dot.setBackground(bgShape(swatch[i],40,
                BladeAppearance.accentName(this).equals(value)?primary():0));
            row.addView(dot,new LinearLayout.LayoutParams(dp(32),dp(32)));
            TextView name=label(colorNames[i],14,textColor(),true);
            LinearLayout.LayoutParams np=new LinearLayout.LayoutParams(0,-2,1f);
            np.leftMargin=dp(13);
            row.addView(name,np);
            if (BladeAppearance.accentName(this).equals(value)) {
                row.addView(label("✓",17,primary(),true));
            }
            row.setClickable(true);
            row.setOnClickListener(v->{
                if(BladeAppearance.CUSTOM.equals(value)) {
                    colorPicker();
                } else {
                    BladeAppearance.setAccentName(this,value);
                    render();
                }
            });
            add(root,row,i==0?10:7);
        }

        add(root,section("АНИМАЦИЯ ПАНЕЛИ"),25);
        String[] descriptions={
            "Волны и орбитальные частицы",
            "Переливающиеся световые ленты",
            "Горизонтальные потоки пакетов",
            "Ритмичные столбцы, зависящие от трафика",
            "Движущаяся кристаллическая сетка"
        };
        for(int i=0;i<BladeAppearance.ANIMATION_NAMES.length;i++){
            final int chosen=i;
            add(root,choice(BladeAppearance.ANIMATION_NAMES[i] + " · " + descriptions[i],
                BladeAppearance.animation(this)==i,
                ()->{BladeAppearance.setAnimation(this,chosen);render();}),i==0?10:7);
        }

        add(root,section("ФОРМА КНОПКИ"),25);
        add(root,choice("Круглая",!BladeAppearance.square(this),()->{
            BladeAppearance.setSquare(this,false);render();
        }),10);
        add(root,choice("Квадратная · скруглённые углы",BladeAppearance.square(this),()->{
            BladeAppearance.setSquare(this,true);render();
        }),7);

        add(root,section("ОТКЛИК"),25);
        LinearLayout toggle = new LinearLayout(this);
        toggle.setGravity(Gravity.CENTER_VERTICAL);
        toggle.setPadding(dp(15),dp(12),dp(15),dp(12));
        toggle.setBackground(bgShape(surfaceColor(),15,border()));
        TextView toggleText=label("Тактильная отдача",15,textColor(),true);
        toggle.addView(toggleText,new LinearLayout.LayoutParams(0,-2,1f));
        Switch haptics=new Switch(this);
        haptics.setChecked(BladeAppearance.haptics(this));
        haptics.setContentDescription("Тактильная отдача");
        haptics.setOnCheckedChangeListener((button,checked)->{
            BladeAppearance.setHaptics(this,checked);
            if(checked) BladeAppearance.haptic(haptics);
        });
        toggle.addView(haptics);
        toggle.setOnClickListener(view->haptics.setChecked(!haptics.isChecked()));
        add(root,toggle,10);
    }

    private View choice(String content, boolean selected, Runnable action) {
        TextView row=label(content + (selected?"   ✓":""),14,
            selected?primary():textColor(),selected);
        row.setPadding(dp(15),dp(15),dp(15),dp(15));
        row.setBackground(bgShape(surfaceColor(),15,selected?primary():border()));
        row.setClickable(true);
        row.setOnClickListener(view -> {
            BladeAppearance.haptic(view);
            action.run();
        });
        return row;
    }

    /** A real HSV palette with live preview and editable 6-digit HEX input. */
    private void colorPicker() {
        final String oldName=BladeAppearance.accentName(this);
        final int oldColor=BladeAppearance.customColor(this);
        final boolean[] confirmed={false};
        BladeAppearance.setAccentName(this,BladeAppearance.CUSTOM);
        LinearLayout panel=column();
        panel.setPadding(dp(18),dp(8),dp(18),dp(4));
        View swatch=new View(this);
        swatch.setBackground(bgShape(oldColor,14,0));
        add(panel,swatch,5);
        swatch.getLayoutParams().height=dp(50);
        EditText hex=new EditText(this);
        hex.setSingleLine(true);
        hex.setTextSize(15);
        hex.setHint("#RRGGBB");
        hex.setText(String.format(java.util.Locale.US,"#%06X",oldColor & 0xffffff));
        add(panel,hex,10);
        String[] names={"Оттенок","Насыщенность","Яркость"};
        float[] hsv=new float[3];
        Color.colorToHSV(oldColor,hsv);
        SeekBar[] sliders=new SeekBar[3];
        boolean[] updating={false};
        for(int i=0;i<3;i++){
            final int index=i;
            add(panel,label(names[i],12,muted(),true),10);
            SeekBar slider=new SeekBar(this);
            slider.setMax(i==0?360:100);
            slider.setProgress(i==0?Math.round(hsv[i]):Math.round(hsv[i]*100));
            sliders[i]=slider;
            add(panel,slider,2);
            slider.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener(){
                @Override public void onStartTrackingTouch(SeekBar seekBar){}
                @Override public void onStopTrackingTouch(SeekBar seekBar){}
                @Override public void onProgressChanged(SeekBar seekBar,int progress,boolean fromUser){
                    if(!fromUser || updating[0])return;
                    hsv[index]=index==0?progress:progress/100f;
                    int color=Color.HSVToColor(hsv);
                    BladeAppearance.setCustomColor(AppearanceActivity.this,color);
                    swatch.setBackground(bgShape(color,14,0));
                    updating[0]=true;
                    hex.setText(String.format(java.util.Locale.US,"#%06X",color&0xffffff));
                    hex.setSelection(hex.length());
                    updating[0]=false;
                    refreshPreview();
                }
            });
        }
        hex.addTextChangedListener(new TextWatcher(){
            @Override public void beforeTextChanged(CharSequence s,int start,int count,int after){}
            @Override public void onTextChanged(CharSequence s,int start,int before,int count){}
            @Override public void afterTextChanged(Editable e){
                if(updating[0])return;
                String value=e.toString().trim();
                if(!value.matches("#?[0-9a-fA-F]{6}"))return;
                try {
                    int color=Color.parseColor(value.startsWith("#")?value:"#"+value);
                    updating[0]=true;
                    Color.colorToHSV(color,hsv);
                    for(int i=0;i<3;i++) sliders[i].setProgress(
                        i==0?Math.round(hsv[i]):Math.round(hsv[i]*100));
                    updating[0]=false;
                    BladeAppearance.setCustomColor(AppearanceActivity.this,color);
                    swatch.setBackground(bgShape(color,14,0));
                    refreshPreview();
                }catch(IllegalArgumentException ignored){}
            }
        });
        AlertDialog dialog=new AlertDialog.Builder(this)
            .setTitle("Свой акцентный цвет")
            .setView(panel)
            .setPositiveButton("Применить",(d,which)->{
                confirmed[0]=true;
                BladeAppearance.setAccentName(this,BladeAppearance.CUSTOM);
                render();
            })
            .setNegativeButton("Отмена",null)
            .create();
        dialog.setOnDismissListener(d->{
            if(!confirmed[0]){
                BladeAppearance.setAccentName(this,oldName);
                BladeAppearance.setCustomColor(this,oldColor);
                render();
            }
        });
        dialog.show();
        refreshPreview();
    }

    private void refreshPreview() {
        if (preview != null) preview.invalidate();
    }
}
