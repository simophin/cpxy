package dev.fanchao.cjkproxy

import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle
import dev.fanchao.CJKProxy

class MainActivity : AppCompatActivity() {
    private var instance: Long = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        instance = CJKProxy.start("oracle-us-west.fanchao.nz", 80, "0.0.0.0", 5000)
    }

    override fun onDestroy() {
        super.onDestroy()

        if (instance != 0L) {
            CJKProxy.stop(instance)
        }
    }
}