package org.anicraft.app;

import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;

import androidx.documentfile.provider.DocumentFile;
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
    private static final int REQUEST_OPEN_FOLDER = 4109;

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
            String extension = "";
            int dot = defaultName == null ? -1 : defaultName.lastIndexOf('.');
            if (dot >= 0 && dot + 1 < defaultName.length()) extension = defaultName.substring(dot + 1).toLowerCase(java.util.Locale.ROOT);
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
            intent.setType(mime == null ? "*/*" : mime);
            intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            if (defaultName != null && !defaultName.trim().isEmpty()) {
                intent.putExtra(Intent.EXTRA_TITLE, new File(defaultName).getName());
            }
            startActivityForResult(intent, REQUEST_CREATE_DOCUMENT);
        });
    }

    /** Choose a folder for an export that creates several files. */
    public void openExportFolder(String title) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            startActivityForResult(intent, REQUEST_OPEN_FOLDER);
        });
    }

    /** Copy every generated frame from the staging directory into the selected SAF folder. */
    public boolean commitFolder(String cachedPath) {
        if (cachedPath == null || cachedPath.isEmpty()) return false;
        String uriText = getSharedPreferences("saf-documents", MODE_PRIVATE)
            .getString(cachedPath, null);
        // A missing mapping is an error for an export target, not a successful no-op:
        // otherwise the SAF placeholder remains a zero-byte file/folder.
        if (uriText == null) return false;
        File source = new File(cachedPath);
        DocumentFile root = DocumentFile.fromTreeUri(this, Uri.parse(uriText));
        return source.isDirectory() && root != null && copyFolderContents(source, root);
    }

    private boolean copyFolderContents(File source, DocumentFile target) {
        File[] children = source.listFiles();
        if (children == null) return false;
        for (File child : children) {
            if (child.isDirectory()) {
                DocumentFile directory = target.findFile(child.getName());
                if (directory == null || !directory.isDirectory()) {
                    directory = target.createDirectory(child.getName());
                }
                if (directory == null || !copyFolderContents(child, directory)) return false;
            } else {
                String extension = "";
                int dot = child.getName().lastIndexOf('.');
                if (dot >= 0 && dot + 1 < child.getName().length()) {
                    extension = child.getName().substring(dot + 1).toLowerCase(java.util.Locale.ROOT);
                }
                String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
                if (mime == null) mime = "application/octet-stream";
                DocumentFile old = target.findFile(child.getName());
                if (old != null && !old.delete()) return false;
                DocumentFile output = target.createFile(mime, child.getName());
                if (output == null || !copyFileToUri(child, output.getUri())) return false;
            }
        }
        return true;
    }

    /** Copy one complete, non-empty staging file and verify the provider's reported size. */
    private boolean copyFileToUri(File source, Uri destination) {
        if (source == null || !source.isFile()) return false;
        long expected = source.length();
        if (expected <= 0) return false;
        long copied = 0;
        try (InputStream input = new java.io.FileInputStream(source);
             OutputStream output = getContentResolver().openOutputStream(destination, "wt")) {
            if (output == null) return false;
            byte[] buffer = new byte[64 * 1024];
            int count;
            while ((count = input.read(buffer)) != -1) {
                output.write(buffer, 0, count);
                copied += count;
            }
            output.flush();
        } catch (Exception error) {
            return false;
        }
        if (copied != expected) return false;
        long actual = queryDocumentSize(destination);
        if (actual == copied) return true;
        // A few SAF providers report stale metadata immediately after a write. Verify the
        // document by reading its bytes back instead of treating a stale zero size as success
        // or failure.
        return countUriBytes(destination) == copied;
    }

    private long countUriBytes(Uri uri) {
        long count = 0;
        try (InputStream input = getContentResolver().openInputStream(uri)) {
            if (input == null) return -1;
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = input.read(buffer)) != -1) count += read;
            return count;
        } catch (Exception ignored) {
            return -1;
        }
    }

    /** Returns -1 when the provider does not expose a size. */
    private long queryDocumentSize(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.SIZE);
                if (column >= 0 && !cursor.isNull(column)) return cursor.getLong(column);
            }
        } catch (Exception ignored) {
            // Some cloud providers do not expose metadata immediately after writing.
        }
        return -1;
    }

    /**
     * Copy a completed cache file to the SAF destination. The URI grant is persisted so
     * ordinary Save can update the same document after the app is restarted.
     */
    public boolean commitDocument(String cachedPath) {
        if (cachedPath == null || cachedPath.isEmpty()) return false;
        String uriText = getSharedPreferences("saf-documents", MODE_PRIVATE)
            .getString(cachedPath, null);
        if (uriText == null) return false;
        return copyFileToUri(new File(cachedPath), Uri.parse(uriText));
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQUEST_OPEN_DOCUMENT && requestCode != REQUEST_CREATE_DOCUMENT && requestCode != REQUEST_OPEN_FOLDER) {
            return;
        }
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            nativeOnFilePicked("");
            return;
        }

        Uri uri = data.getData();
        if (requestCode == REQUEST_OPEN_FOLDER) {
            File documents = new File(getFilesDir(), "documents");
            if (!documents.exists() && !documents.mkdirs()) {
                nativeOnFilePicked("");
                return;
            }
            File folder = new File(documents, "export-" + System.currentTimeMillis());
            try {
                if (!folder.mkdirs()) throw new IOException("Could not create the export staging folder");
                try {
                    getContentResolver().takePersistableUriPermission(uri,
                        data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION));
                } catch (SecurityException ignored) {
                    // Some providers do not offer persistable grants.
                }
                getSharedPreferences("saf-documents", MODE_PRIVATE).edit()
                    .putString(folder.getAbsolutePath(), uri.toString()).apply();
                nativeOnFilePicked(folder.getAbsolutePath());
            } catch (Exception error) {
                nativeOnFilePicked("");
            }
            return;
        }
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

        File documents = new File(getFilesDir(), "documents");
        if (!documents.exists() && !documents.mkdirs()) {
            nativeOnFilePicked("");
            return;
        }
        File cached = new File(documents, "picked-" + System.currentTimeMillis() + "-" + displayName);
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
        // Do not re-apply immersive navigation while Android is showing the IME. On some
        // devices that races the GameActivity/winit text-input request and dismisses the
        // soft keyboard immediately after a text field receives focus.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            WindowInsets rootInsets = getWindow().getDecorView().getRootWindowInsets();
            if (rootInsets != null && rootInsets.isVisible(WindowInsets.Type.ime())) {
                return;
            }
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
