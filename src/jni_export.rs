use crate::client::run_client;
use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong};
use jni::JNIEnv;
use smol::{spawn, Async, Task};
use std::net::TcpListener;

struct Instance(Task<anyhow::Result<()>>);

#[no_mangle]
pub extern "system" fn Java_dev_fanchao_CJKProxy_start(
    env: JNIEnv,
    _: JClass,
    upstream_host: JString,
    upstream_port: jint,
    socks5_host: JString,
    socks5_port: jint,
    socks5_udp_host: JString,
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

    let socks5_udp_host: String = env
        .get_string(socks5_udp_host)
        .expect("To get socks5_udp_host")
        .into();

    let address = format!("{socks5_host}:{socks5_port}");
    let listener = match TcpListener::bind(&address) {
        Ok(v) => smol::net::TcpListener::from(Async::new(v).expect("Async")),
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/Exception",
                format!("Unable to bind address at {address}: {e}"),
            );
            return 0;
        }
    };

    Box::leak(Box::new(Instance(spawn(async move {
        run_client(
            listener,
            upstream_host.as_str(),
            upstream_port.try_into().unwrap(),
            socks5_udp_host.as_str(),
        )
        .await
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
