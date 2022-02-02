package dev.fanchao.cjkproxy

import android.os.Parcelable
import kotlinx.parcelize.Parcelize
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
@Parcelize
data class UpstreamConfig(
    val address: String,
    val accept: List<String>,
    val reject: List<String>,
    @SerialName("match_gfw")
    val matchGfw: Boolean,
    @SerialName("match_networks")
    val matchNetworks: List<String>,
) : Parcelable

@Serializable
@Parcelize
data class ProxyConfiguration(
    @SerialName("socks5_address")
    val socks5Listen: String,
    @SerialName("socks5_udp_host")
    val socks5UdpHost: String,
    val upstreams: Map<String, UpstreamConfig>,
) : Parcelable

@Serializable
@Parcelize
data class ProxyConfigurationPersisted(
    val name: String,
    val config: ProxyConfiguration
) : Parcelable