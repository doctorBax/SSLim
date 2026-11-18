package ru.doctorbax.blade;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.net.Uri;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Explicit, user-initiated version check. No background checks, downloads or installs. */
final class BladeUpdates {
    private static final String API =
        "https://api.github.com/repos/doctorBax/SSLim/releases/latest";
    private static final Pattern VERSION =
        Pattern.compile("(?i)v?(\\d+)\\.(\\d+)\\.(\\d+)");

    private BladeUpdates() {}

    static void check(Activity activity) {
        AtomicBoolean cancelled = new AtomicBoolean(false);
        final AlertDialog progress = BladeMotion.builder(activity)
            .setTitle("Проверка обновлений")
            .setMessage("Проверяем последнюю версию Blade на GitHub…")
            .setNegativeButton("Отмена", (d, which) -> cancelled.set(true))
            .create();
        progress.setOnCancelListener(d -> cancelled.set(true));
        BladeMotion.show(progress);
        new Thread(() -> {
            try {
                JSONObject release = latestRelease();
                String tag = release.getString("tag_name");
                String url = release.getString("html_url");
                String body = release.optString("body","");
                String local = activity.getPackageManager()
                    .getPackageInfo(activity.getPackageName(), 0).versionName;
                boolean apkFound = false;
                JSONArray assets = release.optJSONArray("assets");
                if (assets != null) for (int i=0;i<assets.length();i++) {
                    if ("Blade.apk".equalsIgnoreCase(assets.getJSONObject(i).optString("name"))) {
                        apkFound = true;
                        break;
                    }
                }
                boolean hasApk = apkFound;
                int comparison = compareVersions(tag, local);
                activity.runOnUiThread(() -> {
                    progress.dismiss();
                    if (activity.isFinishing() || activity.isDestroyed() || cancelled.get()) return;
                    if (comparison <= 0) {
                        BladeMotion.show(BladeMotion.builder(activity)
                            .setTitle("Обновлений нет")
                            .setMessage("Установлена: " + local
                                + "\nПоследний релиз: " + tag
                                + "\n\nНовые версии проверяются только вручную.")
                            .setPositiveButton("Понятно", null).create());
                    } else if (!hasApk) {
                        BladeMotion.show(BladeMotion.builder(activity)
                            .setTitle("Найден новый релиз")
                            .setMessage("Версия " + tag + " опубликована, но Android APK "
                                + "в составе релиза не найден.")
                            .setPositiveButton("Понятно",null).create());
                    } else {
                        String notes = body.length() > 1600
                            ? body.substring(0,1600) + "…" : body;
                        BladeMotion.show(BladeMotion.builder(activity)
                            .setTitle("Доступна версия " + tag)
                            .setMessage("Установлена: " + local
                                + "\n\n" + notes
                                + "\n\nBlade не скачивает и не устанавливает APK автоматически.")
                            .setPositiveButton("Открыть релиз", (d, which) -> {
                                activity.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
                            })
                            .setNegativeButton("Позже", null).create());
                    }
                });
            } catch (Exception error) {
                activity.runOnUiThread(() -> {
                    progress.dismiss();
                    if (activity.isFinishing() || activity.isDestroyed() || cancelled.get()) return;
                    BladeMotion.show(BladeMotion.builder(activity)
                        .setTitle("Не удалось проверить обновления")
                        .setMessage("Нет ответа от GitHub или неверный формат релиза.\n\n"
                            + error.getClass().getSimpleName()
                            + (error.getMessage() == null ? "" : ": " + error.getMessage()))
                        .setPositiveButton("Закрыть", null).create());
                });
            }
        }, "blade-check-updates").start();
    }

    private static JSONObject latestRelease() throws Exception {
        HttpURLConnection conn = (HttpURLConnection)new URL(API).openConnection();
        try {
            conn.setConnectTimeout(8000);
            conn.setReadTimeout(8000);
            conn.setRequestProperty("Accept", "application/vnd.github+json");
            conn.setRequestProperty("User-Agent", "Blade-Android-Manual-Update-Check");
            conn.setUseCaches(false);
            if(conn.getResponseCode()!=200) {
                throw new java.io.IOException("GitHub HTTP " + conn.getResponseCode());
            }
            try(InputStream stream=conn.getInputStream();
                ByteArrayOutputStream output=new ByteArrayOutputStream()) {
                byte[] buffer=new byte[4096];
                int n;
                while((n=stream.read(buffer))!=-1) {
                    if(output.size()+n>256*1024) {
                        throw new java.io.IOException("Ответ GitHub слишком большой");
                    }
                    output.write(buffer,0,n);
                }
                return new JSONObject(new String(output.toByteArray(),StandardCharsets.UTF_8));
            }
        } finally {
            conn.disconnect();
        }
    }

    static int compareVersions(String remote, String local) {
        int[] r=parts(remote);
        int[] l=parts(local);
        for(int i=0;i<3;i++) {
            if(r[i]!=l[i])return r[i]>l[i]?1:-1;
        }
        return 0;
    }

    private static int[] parts(String text) {
        Matcher matcher=VERSION.matcher(text);
        if(!matcher.find())throw new IllegalArgumentException("Неизвестная версия: "+text);
        return new int[]{Integer.parseInt(matcher.group(1)),
            Integer.parseInt(matcher.group(2)),Integer.parseInt(matcher.group(3))};
    }
}
