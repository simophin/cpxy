package dev.fanchao;

import androidx.annotation.NonNull;

public class CJKProxy {
    public native static void verifyConfig(@NonNull String config) throws Exception;

    public native static long start(@NonNull String config) throws Exception;

    public native static void stop(long instance);

    static {
        System.loadLibrary("proxy");
    }
}
