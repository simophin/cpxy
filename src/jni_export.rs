use crate::client::run_client;
use async_broadcast::{broadcast, Sender};
use async_std::task::{spawn, JoinHandle};
use jni::objects::{JClass, JString};
use jni::sys::{jlong, jshort};
use jni::JNIEnv;
use std::net::TcpListener;

struct Instance {
    handle: JoinHandle<anyhow::Result<()>>,
    quit_tx: Sender<()>,
}

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

    let address = format!("{socks5_host}:{socks5_port}");
    let listener = match TcpListener::bind(&address) {
        Ok(v) => async_std::net::TcpListener::from(v),
        Err(e) => {
            let _ = env.throw_new(
                "java/lang/Exception",
                format!("Unable to bind address at {address}: {e}"),
            );
            return 0;
        }
    };

    let (quit_tx, quit_rx) = broadcast(10);

    Box::leak(Box::new(Instance {
        handle: spawn(async move {
            run_client(
                listener,
                upstream_host.as_str(),
                upstream_port as u16,
                quit_rx,
            )
            .await
        }),
        quit_tx,
    })) as *mut Instance as jlong
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
    spawn(async move {
        let _ = handle.quit_tx.broadcast(()).await;
        let _ = handle.handle.await;
    });
}
