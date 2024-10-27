FROM rust:1.82-alpine3.20 as builder

WORKDIR /usr/src/rust_blogging_project
#RUN apt-get update && apt-get install -y libssl-dev && rm -rf /var/lib/apt/lists/*
RUN apk add --no-cache openssl-dev openssl-libs-static musl-dev
COPY . .
RUN cargo install --path .


FROM alpine:3.20 as final

RUN apk add --no-cache libssl3
COPY --from=builder /usr/local/cargo/bin/rust_blogging_project /usr/local/bin/rust_blogging_project
WORKDIR /usr/share/rust_blogging_project
RUN touch posts.db && mkdir assets && mkdir assets/avatars && mkdir assets/images
EXPOSE 3000
CMD ["rust_blogging_project"]
