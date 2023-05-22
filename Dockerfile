FROM node:lts

WORKDIR /app

COPY ./web ./
RUN yarn install --frozen-lockfile --network-timeout 1000000 && yarn build

FROM rust:buster

WORKDIR /app

# Download rust deps
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir -p web/build src && touch src/lib.rs && cargo build --release

# Do the real build now
COPY . ./
COPY --from=0 /app/build web/build

RUN cargo build --release -p cpxy

RUN mv -v target/$TARGET/release/cpxy ./

FROM debian:buster
COPY --from=1 /app/cpxy /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/cpxy"]