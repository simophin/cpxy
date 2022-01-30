package dev.fanchao.cjkproxy

import android.os.Looper
import android.util.Log
import android.widget.Toast
import dev.fanchao.CJKProxy
import io.reactivex.rxjava3.core.Observable
import io.reactivex.rxjava3.subjects.BehaviorSubject
import java.io.Closeable

sealed interface ProxyState {
    data class Running(val instance: Long) : ProxyState
    object Stopped : ProxyState
}

class ProxyInstances : Closeable {
    private var instances = emptyMap<ProxyConfiguration, ProxyState>()
    private val notification = BehaviorSubject.createDefault(Unit)

    fun observeInstances(): Observable<Map<ProxyConfiguration, ProxyState>> {
        return notification.map { instances }
    }

    @Synchronized
    fun updateInstances(configurations: Set<ProxyConfiguration>) {
        val newInstances = hashMapOf<ProxyConfiguration, ProxyState>()
        // Check for additions
        for (c in configurations) {
            val existing = instances[c]
            newInstances[c] = if (existing is ProxyState.Running) {
                existing
            }
            else {
                try {
                    ProxyState.Running(CJKProxy.start(c.remoteHost, c.remotePort, c.socksHost, c.socksPort, c.socksUdpHost)).apply {
                        Log.d("ProxyInstances", "Started $c")
                    }
                } catch (ec: Throwable) {
                    if (Looper.getMainLooper() == Looper.myLooper()) {
                        Toast.makeText(App.instance, "Error starting proxy: ${ec.message}", Toast.LENGTH_LONG).show()
                    }

                    ProxyState.Stopped
                }
            }
        }

        // Check for removal
        for ((c, state) in instances) {
            if (!newInstances.containsKey(c) && state is ProxyState.Running) {
                CJKProxy.stop(state.instance)
                Log.d("ProxyInstances", "Stopped $c")
            }
        }

        instances = newInstances
        notification.onNext(Unit)
    }

    fun pickNewLocalPort(): Int {
        for (i in 5000 until 6000) {
            if (instances.keys.find { it.socksPort == i } == null) {
                return i
            }
        }

        return 6001
    }

    @Synchronized
    override fun close() {
        for (state in instances.values) {
            if (state is ProxyState.Running) {
                CJKProxy.stop(state.instance);
            }
        }
        instances = emptyMap()
    }
}