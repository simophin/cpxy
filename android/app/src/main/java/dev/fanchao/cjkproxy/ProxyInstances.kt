package dev.fanchao.cjkproxy

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
                    ProxyState.Running(CJKProxy.start(c.remoteHost, c.remotePort, c.socksHost, c.socksPort))
                } catch (ec: Throwable) {
                    ProxyState.Stopped
                }
            }
        }

        // Check for removal
        for ((c, state) in instances) {
            if (!newInstances.containsKey(c) && state is ProxyState.Running) {
                CJKProxy.stop(state.instance)
            }
        }

        instances = newInstances
        notification.onNext(Unit)
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