FROM rust:1.40 as build

WORKDIR /code
# RUN apt-get update && apt-get install -y musl musl-dev musl-tools
# RUN ln -s /usr/bin/musl-gcc /usr/local/bin/$target-gcc && ln -s /usr/bin/musl-gcc /usr/local/bin/$platform-linux-musl-gcc

ARG platform=x86_64
ARG target=$platform-unknown-linux-musl
RUN rustup target add $target
COPY ./build/cargo_conf ./.cargo/config

COPY ./Cargo.toml ./
COPY ./Cargo.lock ./
COPY ./src/dummy.rs ./src/dummy.rs

RUN cargo build --release --locked --target $target --bin dummy

COPY ./src/ ./src

RUN cargo build --release --locked --target $target && \
    strip target/$target/release/tcp_multiplexer && \
    mv target/$target/release/tcp_multiplexer target

FROM alpine as done

COPY --from=build /code/target/tcp_multiplexer /app/tcp_multiplexer

CMD [ "/app/tcp_multiplexer" ]

FROM done

RUN ["/app/tcp_multiplexer", "test-binary"]

FROM done