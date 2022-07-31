FROM node:lts
RUN mkdir /app

COPY ./web /app/
RUN cd /app && yarn install && yarn build

FROM rust

RUN mkdir /app
COPY . /app/
COPY --from=0 /app/build /app/web/build

RUN cd /app && cargo build --release -p cjk-proxy

FROM ubuntu
COPY --from=1 /app/target/release/proxy /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/proxy"]