package com.tusher.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Build
import com.tusher.app.service.TusherForegroundService

class TusherApplication : Application() {

    override fun onCreate() {
        super.onCreate()
        createNotificationChannels()
    }

    private fun createNotificationChannels() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val syncChannel = NotificationChannel(
                TusherForegroundService.CHANNEL_ID,
                "T.U.S.H.E.R Sync",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Background file synchronization status"
            }

            val alertChannel = NotificationChannel(
                CHANNEL_ALERTS,
                "Sync Alerts",
                NotificationManager.IMPORTANCE_DEFAULT
            ).apply {
                description = "Sync completion and conflict alerts"
            }

            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannels(listOf(syncChannel, alertChannel))
        }
    }

    companion object {
        const val CHANNEL_ALERTS = "tusher_alerts"
    }
}
