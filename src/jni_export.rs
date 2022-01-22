use crate::client::run_client;
use async_std::task::{spawn, JoinHandle};
use jni::objects::{JClass, JString};
use jni::sys::{jlong, jshort};
use jni::JNIEnv;

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_start(
    env: JNIEnv,
    _: JClass,
    upstream_host: JString,
    upstream_port: jshort,
    socks5_host: JString,
    socks5_port: jshort,
) -> jlong {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_min_level(log::Level::Info)
            .with_tag("proxy_rust"),
    );

    let upstream_host: String = env
        .get_string(upstream_host)
        .expect("To get host string")
        .into();
    let socks5_host: String = env
        .get_string(socks5_host)
        .expect("To get host string")
        .into();

    let handle: &'static mut JoinHandle<_> = Box::leak(Box::new(spawn(async move {
        run_client(
            format!("{socks5_host}:{socks5_port}"),
            upstream_host.as_str(),
            upstream_port as u16,
        )
        .await
    })));

    handle as *mut JoinHandle<anyhow::Result<()>> as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_stop(_: JNIEnv, _: JClass, instance: jlong) {
    let handle: Box<JoinHandle<anyhow::Result<()>>> =
        unsafe { Box::from_raw(instance as *mut JoinHandle<_>) };
    let _ = handle.cancel();
}
