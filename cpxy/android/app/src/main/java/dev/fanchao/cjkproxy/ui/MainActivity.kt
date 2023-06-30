package dev.fanchao.cjkproxy.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PermissionInfo
import android.os.Bundle
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import dev.fanchao.cjkproxy.Service
import dev.fanchao.cjkproxy.databinding.ActivityMainBinding
import io.reactivex.rxjava3.disposables.Disposable

class MainActivity : AppCompatActivity() {
    private lateinit var binding: ActivityMainBinding
    private var disposable: Disposable? = null

    private val requestPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) {
        if (it) {
            startService(Intent(this, Service::class.java))
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        binding.start.setOnClickListener {
            requestPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }

        binding.stop.setOnClickListener {
            stopService(Intent(this, Service::class.java))
        }
    }

    private fun ActivityMainBinding.updateState() {
        start.isEnabled = Service.runningPort == null
        stop.isEnabled = Service.runningPort != null
        admin.isEnabled = Service.runningPort != null
        Service.runningPort?.let { port ->
            admin.setOnClickListener {
                startActivity(Intent(this@MainActivity, AdminActivity::class.java)
                        .putExtra("port", port)
                )
            }
        }
    }

    override fun onStart() {
        super.onStart()

        disposable = Service.notifications
            .startWithItem(Unit)
            .subscribe {
                binding.updateState()
            }
    }

    override fun onStop() {
        super.onStop()

        disposable?.dispose()
    }
}