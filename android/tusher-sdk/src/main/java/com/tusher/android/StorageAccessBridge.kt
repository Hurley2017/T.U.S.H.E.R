package com.tusher.android

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import androidx.documentfile.provider.DocumentFile
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream

/**
 * Bridges Android's Storage Access Framework (SAF) content:// URIs with local filesystem paths.
 * Allows T.U.S.H.E.R to synchronize Android folders (SD cards, Downloads, Documents)
 * without requiring root permissions.
 */
class StorageAccessBridge(private val context: Context) {

    /**
     * Resolves a tree document URI granted via Intent.ACTION_OPEN_DOCUMENT_TREE
     * and ensures a local sync directory exists in app storage for caching and staging.
     */
    fun resolveSyncPath(treeUri: Uri, folderId: String): File {
        val root = DocumentFile.fromTreeUri(context, treeUri)
            ?: throw IllegalArgumentException("Invalid tree URI: $treeUri")

        val localMirror = File(context.getExternalFilesDir(null), "mesh_folders/$folderId").apply {
            mkdirs()
        }

        return localMirror
    }

    /**
     * Copies a modified file from local staging/sync mirror back into the SAF destination URI.
     */
    fun exportToSaf(localFile: File, targetDirUri: Uri, mimeType: String = "application/octet-stream") {
        val dir = DocumentFile.fromTreeUri(context, targetDirUri)
            ?: throw IllegalArgumentException("Target directory not found: $targetDirUri")

        var targetDoc = dir.findFile(localFile.name)
        if (targetDoc == null) {
            targetDoc = dir.createFile(mimeType, localFile.name)
                ?: throw IllegalStateException("Failed to create SAF document for ${localFile.name}")
        }

        context.contentResolver.openOutputStream(targetDoc.uri)?.use { outStream ->
            localFile.inputStream().use { inStream ->
                inStream.copyTo(outStream)
            }
        }
    }

    /**
     * Imports a file from a SAF URI into the local synchronization mirror folder.
     */
    fun importFromSaf(safUri: Uri, destinationFile: File) {
        destinationFile.parentFile?.mkdirs()
        context.contentResolver.openInputStream(safUri)?.use { inStream ->
            FileOutputStream(destinationFile).use { outStream ->
                inStream.copyTo(outStream)
            }
        }
    }
}
