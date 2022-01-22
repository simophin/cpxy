package dev.fanchao;

import androidx.annotation.NonNull;

public class CJKProxy {
    public native static long start(@NonNull String upstreamHost, short upstreamPort,
                                    @NonNull String socks5Host, short socks5Port) throws Exception;

    public native static void stop(long instance);

    static {
        System.loadLibrary("proxy");
    }
}
