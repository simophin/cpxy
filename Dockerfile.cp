# This Dockerfile will only copy the built files
FROM alpine:latest

ARG TARGETARCH

COPY cpxy.* /app/
COPY docker .

RUN cp /app/cpxy.$(sh TARGETARCH=$TARGETARCH docker/platform.sh) /usr/local/bin/ &&
    rm -rfv /app

EXPOSE 80/tcp
EXPOSE 3000/udp
ENTRYPOINT ["/usr/local/bin/cpxy"]