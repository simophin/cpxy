FROM node:lts

WORKDIR /app

COPY ./web ./
RUN yarn install --frozen-lockfile --network-timeout 1000000 && yarn build

FROM messense/rust-musl-cross:x86_64-musl

WORKDIR /app

ARG TARGET=x86_64-unknown-linux-musl 

# RUN rustup target add $TARGET

# Download rust deps
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p web/build src && touch src/lib.rs && cargo build --target=$TARGET --release

# Do the real build now
COPY . ./
COPY --from=0 /app/build web/build

RUN cargo build --target=$TARGET --release -p cpxy

RUN mv -v target/$TARGET/release/cpxy ./

FROM scratch
COPY --from=1 /app/cpxy /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/cpxy"]