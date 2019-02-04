#pragma once

#include <limits.h>
#include <sys/types.h>

#define MAX_CONNECTIONS 256
#define MAX_SOCKETS (MAX_CONNECTIONS + 2)
#define HEADER_SIZE 3
#define MAX_PAYLOAD_SIZE (PIPE_BUF - HEADER_SIZE)

#define connection_t uint8_t