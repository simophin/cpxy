FROM rust

RUN mkdir /app
WORKDIR /app
COPY . ./

RUN mkdir -p web/build && \
    cargo build --release -p cjk-proxy

FROM ubuntu
COPY --from=0 /app/target/release/proxy /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
CMD ["/usr/local/bin/proxy"]