FROM rust:1.40 as build

WORKDIR /code
RUN apt-get update && apt-get install -y musl musl-dev musl-tools

ARG platform=x86_64
ARG target=$platform-unknown-linux-musl
RUN rustup target add $target

COPY ./Cargo.toml ./
COPY ./Cargo.lock ./
COPY ./src/dummy.rs ./src/dummy.rs

RUN cargo build --release --target $target

COPY ./src/ ./src

RUN cargo build --release --target $target

RUN strip target/$target/release/tcp_multiplexer && mv target/$target/release/tcp_multiplexer target

FROM alpine as done

COPY --from=build /code/target/tcp_multiplexer /app/tcp_multiplexer

CMD [ "/app/tcp_multiplexer" ]

FROM done

RUN ["/app/tcp_multiplexer", "--test-binary"]

FROM done