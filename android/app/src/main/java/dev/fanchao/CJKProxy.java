package dev.fanchao;

import androidx.annotation.NonNull;

public class CJKProxy {
    public native static long start(@NonNull String configFilePath) throws Exception;

    public native static int getPort(long instance);

    public native static void stop(long instance);

    static {
        System.loadLibrary("proxy");
    }
}
