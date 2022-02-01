use crate::client::run_client;
use crate::config::ClientConfig;
use jni::objects::{JClass, JString};
use jni::sys::jlong;
use jni::JNIEnv;
use smol::{spawn, Async, Task};
use std::net::TcpListener;
use std::sync::Arc;

struct Instance(Task<anyhow::Result<()>>);

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_verifyConfig(
    env: JNIEnv,
    _: JClass,
    config: JString,
) {
    let config: String = env.get_string(config).expect("To get config string").into();
    if let Err(e) = serde_yaml::from_str::<ClientConfig>(config.as_str()) {
        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_start(
    env: JNIEnv,
    _: JClass,
    config: JString,
) -> jlong {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_min_level(log::Level::Info)
            .with_tag("proxy_rust"),
    );

    let config: String = env.get_string(config).expect("To get config string").into();

    let config: ClientConfig = match serde_yaml::from_str(config.as_str()) {
        Ok(c) => c,
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                format!("Error parsing config: {e}"),
            );
            return 0;
        }
    };

    let listener = match TcpListener::bind(config.socks5_address.to_string()) {
        Ok(v) => smol::net::TcpListener::from(Async::new(v).expect("Async")),
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/Exception",
                format!("Unable to bind address at {}: {e}", config.socks5_address),
            );
            return 0;
        }
    };

    Box::leak(Box::new(Instance(spawn(async move {
        run_client(listener, Arc::new(config)).await
    })))) as *mut Instance as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_stop(env: JNIEnv, _: JClass, instance: jlong) {
    if instance == 0 {
        let _ = env.throw_new(
            "java.lang.NullPointerException",
            "Pointer to proxy can not be null",
        );
        return;
    }
    let handle: Box<Instance> = unsafe { Box::from_raw(instance as *mut Instance) };
    drop(handle)
}
