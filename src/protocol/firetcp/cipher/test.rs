use async_net::TcpStream;
use chacha20::ChaCha20;
use cipher::KeyIvInit;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use smol::future::block_on;

use crate::{test::echo_tcp_server, utils::new_vec_uninitialised};

use super::*;

async fn test_echo_message(
    input: &[u8],
    r: &mut (impl AsyncRead + Unpin),
    w: &mut (impl AsyncWrite + Unpin),
) {
    w.write_all(input).await.expect("To write first message");

    let mut received = new_vec_uninitialised(input.len());
    r.read_exact(&mut received)
        .await
        .expect("To read echoed message");

    assert_eq!(input, &received);
}

#[test]
fn cipher_rw_without_establish_cipher_works() {
    block_on(async move {
        let (_server_task, addr) = echo_tcp_server().await;

        let init_key: [u8; 32] = rand::random();
        let init_nonce: [u8; 12] = rand::random();

        let (client_r, client_w) = TcpStream::connect(addr).await.expect("To connect").split();
        let mut client_r = CipherRead::<_, _, ChaCha20>::new(
            client_r,
            128,
            ChaCha20::new_from_slices(&init_key, &init_nonce).unwrap(),
        );
        let mut client_w = CipherWrite::<_, _, ChaCha20>::new(
            client_w,
            128,
            ChaCha20::new_from_slices(&init_key, &init_nonce).unwrap(),
        );

        test_echo_message(&rand::random::<[u8; 28]>(), &mut client_r, &mut client_w).await;
        test_echo_message(&rand::random::<[u8; 128]>(), &mut client_r, &mut client_w).await;
    })
}

#[test]
fn cipher_rw_with_establish_cipher_works() {
    block_on(async move {
        let (_server_task, addr) = echo_tcp_server().await;

        let init_key: [u8; 32] = rand::random();
        let init_nonce: [u8; 12] = rand::random();

        let (client_r, client_w) = TcpStream::connect(addr).await.expect("To connect").split();
        let mut client_r = CipherRead::<_, _, ChaCha20>::new(
            client_r,
            128,
            ChaCha20::new_from_slices(&init_key, &init_nonce).unwrap(),
        );
        let mut client_w = CipherWrite::<_, _, ChaCha20>::new(
            client_w,
            128,
            ChaCha20::new_from_slices(&init_key, &init_nonce).unwrap(),
        );

        let key: [u8; 32] = rand::random();
        let nonce: [u8; 12] = rand::random();

        test_echo_message(&rand::random::<[u8; 28]>(), &mut client_r, &mut client_w).await;
        client_r
            .set_establish_cipher(ChaCha20::new_from_slices(&key, &nonce).unwrap())
            .unwrap();
        client_w
            .set_establish_cipher(ChaCha20::new_from_slices(&key, &nonce).unwrap())
            .unwrap();

        test_echo_message(&rand::random::<[u8; 128]>(), &mut client_r, &mut client_w).await;
        test_echo_message(&rand::random::<[u8; 256]>(), &mut client_r, &mut client_w).await;
    })
}
