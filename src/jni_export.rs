use crate::controller::run_controller;
use crate::io::bind_tcp;
use smol::{block_on, spawn, Task};
use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong};
use jni::JNIEnv;
use std::path::Path;

struct Instance(u16, Task<anyhow::Result<()>>);

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_start(
    env: JNIEnv,
    _: JClass,
    config_path: JString,
) -> jlong {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_min_level(log::Level::Info)
            .with_tag("proxy_rust"),
    );

    let config_path: String = env
        .get_string(config_path)
        .expect("To get config string")
        .into();

    let listener = match block_on(bind_tcp(&"127.0.0.1:0".try_into().unwrap())) {
        Ok(v) => v,
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/Exception",
                format!("Unable to bind controller socket: {e}"),
            );
            return 0;
        }
    };

    let port = match listener.local_addr() {
        Ok(v) => v.port(),
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/Exception",
                format!("Unable to get bound port: {e}"),
            );
            return 0;
        }
    };

    Box::leak(Box::new(Instance(
        port,
        spawn(async move { run_controller(listener, Path::new(&config_path)).await }),
    ))) as *mut Instance as jlong
}

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_getPort(
    _: JNIEnv,
    _: JClass,
    instance: jlong,
) -> jint {
    let handle: Box<Instance> = unsafe { Box::from_raw(instance as *mut Instance) };
    let result = handle.0;
    let _ = Box::leak(handle);
    result as jint
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
