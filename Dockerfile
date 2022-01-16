FROM rust:alpine3.14

RUN mkdir /app
WORKDIR /app
COPY . ./

RUN cargo build --release

FROM alpine:3.14
COPY --from=0 /app/target/release/cjk-proxy /usr/local/bin/

ENTRYPOINT /usr/local/bin/cjk-proxy
EXPOSE 8000
CMD server