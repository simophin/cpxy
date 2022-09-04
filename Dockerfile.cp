# This Dockerfile will only copy the built files
FROM alpine:latest

ARG TARGETARCH
ARG APP_SRC

COPY APP_SRC /usr/local/bin/

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/cpxy"]