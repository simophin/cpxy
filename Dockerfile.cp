# This Dockerfile will only copy the built files
FROM alpine:latest

ARG TARGETARCH

WORKDIR /app

COPY *musl .

RUN cp $(sh docker/platform.sh)/release/cpxy /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/cpxy"]