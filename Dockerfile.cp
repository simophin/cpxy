# This Dockerfile will only copy the built files
FROM debian:latest

ARG APP_SRC

COPY ${APP_SRC} /usr/local/bin/

RUN chmod +x /usr/local/bin/cpxy

ENTRYPOINT ["/usr/local/bin/cpxy"]