package dev.fanchao.cjkproxy

import android.app.Application

class App : Application() {
    val proxyInstances = ProxyInstances()

    companion object {
        lateinit var instance: App
        private set
    }
}