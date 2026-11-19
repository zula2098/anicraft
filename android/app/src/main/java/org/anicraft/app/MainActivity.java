package org.anicraft.app;

import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;

import com.google.androidgamesdk.GameActivity;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * Android host for the Rust/eframe native library.
 *
 * The Storage Access Framework is used for document access: AniCraft never needs
 * broad filesystem permissions and can open files from local or cloud providers.
 */
public final class MainActivity extends GameActivity {
    private static final int REQUEST_OPEN_DOCUMENT = 4107;
    private static final int REQUEST_CREATE_DOCUMENT = 4108;

    private static native void nativeOnFilePicked(String cachedPath);

    public void openDocument(String title) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("*/*");
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(intent, REQUEST_OPEN_DOCUMENT);
        });
    }

    /** Ask the user where a newly-created project or export should be written. */
    public void createDocument(String title, String defaultName) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("*/*");
            intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            if (defaultName != null && !defaultName.trim().isEmpty()) {
                intent.putExtra(Intent.EXTRA_TITLE, new File(defaultName).getName());
            }
            startActivityForResult(intent, REQUEST_CREATE_DOCUMENT);
        });
    }

    /**
     * Copy a completed cache file to the SAF destination. The URI grant is persisted so
     * ordinary Save can update the same document after the app is restarted.
     */
    public boolean commitDocument(String cachedPath) {
        if (cachedPath == null || cachedPath.isEmpty()) return false;
        String uriText = getSharedPreferences("saf-documents", MODE_PRIVATE)
            .getString(cachedPath, null);
        if (uriText == null) return true; // Not a file created through the Android picker.
        Uri uri = Uri.parse(uriText);
        File source = new File(cachedPath);
        if (!source.isFile()) return false;
        try (InputStream input = new java.io.FileInputStream(source);
             OutputStream output = getContentResolver().openOutputStream(uri, "wt")) {
            if (output == null) return false;
            byte[] buffer = new byte[64 * 1024];
            int count;
            while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
            output.flush();
            return true;
        } catch (Exception error) {
            return false;
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQUEST_OPEN_DOCUMENT && requestCode != REQUEST_CREATE_DOCUMENT) {
            return;
        }
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            nativeOnFilePicked("");
            return;
        }

        Uri uri = data.getData();
        if (requestCode == REQUEST_CREATE_DOCUMENT) {
            String displayName = "Untitled.anicraft";
            try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    if (column >= 0) {
                        String name = cursor.getString(column);
                        if (name != null && !name.trim().isEmpty()) displayName = new File(name).getName();
                    }
                }
            } catch (Exception ignored) {
                // A provider may not expose a display name; the suggested name remains usable.
            }
            File documents = new File(getFilesDir(), "documents");
            if (!documents.exists() && !documents.mkdirs()) {
                nativeOnFilePicked("");
                return;
            }
            File cached = new File(documents, "saf-" + System.currentTimeMillis() + "-" + displayName.replaceAll("[^A-Za-z0-9._-]", "_"));
            try {
                if (!cached.createNewFile()) throw new IOException("Could not create the temporary document");
                try {
                    getContentResolver().takePersistableUriPermission(uri,
                        Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
                } catch (SecurityException ignored) {
                    // Some document providers grant access only for the current process.
                }
                getSharedPreferences("saf-documents", MODE_PRIVATE).edit()
                    .putString(cached.getAbsolutePath(), uri.toString()).apply();
                nativeOnFilePicked(cached.getAbsolutePath());
            } catch (Exception error) {
                if (cached.exists()) cached.delete();
                nativeOnFilePicked("");
            }
            return;
        }
        String displayName = "selected-file";
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    String name = cursor.getString(column);
                    if (name != null && !name.trim().isEmpty()) {
                        displayName = new File(name).getName().replaceAll("[^A-Za-z0-9._-]", "_");
                    }
                }
            }
        } catch (Exception ignored) {
            // Some document providers don't expose metadata; the content is still readable.
        }

        File cached = new File(getCacheDir(), "picked-" + System.currentTimeMillis() + "-" + displayName);
        try (InputStream input = getContentResolver().openInputStream(uri);
             OutputStream output = new FileOutputStream(cached)) {
            if (input == null) {
                throw new IOException("The selected document could not be opened");
            }
            byte[] buffer = new byte[64 * 1024];
            int count;
            while ((count = input.read(buffer)) != -1) {
                output.write(buffer, 0, count);
            }
            output.flush();
            nativeOnFilePicked(cached.getAbsolutePath());
        } catch (Exception error) {
            if (cached.exists()) {
                //noinspection ResultOfMethodCallIgnored
                cached.delete();
            }
            nativeOnFilePicked("");
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        applyImmersiveMode();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            applyImmersiveMode();
        }
    }

    private void applyImmersiveMode() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                controller.setSystemBarsBehavior(
                    WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                );
            }
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            );
        }
    }
}
