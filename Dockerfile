FROM rust

RUN mkdir /app
WORKDIR /app
COPY . ./

RUN cargo build --release -p cjk-proxy --no-default-features

FROM ubuntu
COPY --from=0 /app/target/release/cjk-proxy /usr/local/bin/

EXPOSE 8000
CMD ["/usr/local/bin/cjk-proxy", "server"]