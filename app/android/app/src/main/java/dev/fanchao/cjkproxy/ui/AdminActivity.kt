package dev.fanchao.cjkproxy.ui

import android.annotation.SuppressLint
import android.os.Bundle
import android.view.ViewGroup
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity

class AdminActivity : AppCompatActivity() {
    private lateinit var webView: WebView

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val view = WebView(this).apply {
            settings.javaScriptEnabled = true
            loadUrl("http://127.0.0.1:" + intent.getIntExtra("port", 0))
        }

        setContentView(view, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        webView = view
    }

    override fun onBackPressed() {
        if (webView.canGoBack()) {
            webView.goBack();
            return
        }
        super.onBackPressed()
    }

    override fun onPause() {
        super.onPause()

        webView.onPause()
    }

    override fun onResume() {
        super.onResume()

        webView.onResume()
    }

    override fun onDestroy() {
        super.onDestroy()

        webView.destroy()
    }
}