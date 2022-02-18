package dev.fanchao.cjkproxy

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.widget.Toast
import androidx.core.app.NotificationCompat
import dev.fanchao.CJKProxy
import dev.fanchao.cjkproxy.ui.AdminActivity
import dev.fanchao.cjkproxy.ui.MainActivity
import io.reactivex.rxjava3.subjects.PublishSubject
import java.io.File

class Service : android.app.Service() {
    override fun onBind(p0: Intent?): IBinder? = null

    private fun createNotificationChannel() {
        // Create the NotificationChannel, but only on API 26+ because
        // the NotificationChannel class is new and not in the support library
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val importance = NotificationManager.IMPORTANCE_DEFAULT
            val channel = NotificationChannel("default", "default", importance)
            // Register the channel with the system
            val notificationManager: NotificationManager =
                    getSystemService(NOTIFICATION_SERVICE) as NotificationManager
            notificationManager.createNotificationChannel(channel)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (instance == 0L) {
            try {
                instance = CJKProxy.start(File(filesDir, "config.yaml").absolutePath)
                runningPort = CJKProxy.getPort(instance)
            } catch (e: Exception) {
                stopService(Intent(this, javaClass))
                Toast.makeText(this, "Error starting server: ${e.message}", Toast.LENGTH_SHORT)
                        .show()
                return START_NOT_STICKY
            }

            createNotificationChannel()
            val notification =
                    NotificationCompat.Builder(this, "default")
                            .setSmallIcon(R.drawable.ic_add)
                            .setContentTitle("Proxy running")
                            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
                            .setContentIntent(
                                    PendingIntent.getActivity(
                                            this,
                                            0,
                                            Intent(this, MainActivity::class.java),
                                            PendingIntent.FLAG_IMMUTABLE
                                    )
                            )
                            .also { builder ->
                                if (runningPort != null) {
                                    builder.addAction(
                                            R.drawable.ic_add,
                                            "Admin",
                                            PendingIntent.getActivity(
                                                    this,
                                                    0,
                                                    Intent(this, AdminActivity::class.java)
                                                            .putExtra("port", runningPort!!),
                                                    PendingIntent.FLAG_IMMUTABLE
                                            )
                                    )
                                }
                            }
                            .build()

            startForeground(1, notification)
            notifications.onNext(Unit)
            return START_STICKY
        }
        return super.onStartCommand(intent, flags, startId)
    }

    override fun onDestroy() {
        super.onDestroy()

        if (instance != 0L) {
            CJKProxy.stop(instance)
            instance = 0L
            runningPort = null
            notifications.onNext(Unit)
        }
    }

    companion object {
        private var instance: Long = 0

        var runningPort: Int? = null

        val notifications = PublishSubject.create<Unit>()
    }
}
