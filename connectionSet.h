#pragma once

#include <sys/socket.h>
#include <unistd.h>
#include <assert.h>

#include "constants.h"

enum connection_status {
    CONNECTION_STATUS_CLOSED = 0,
    CONNECTION_STATUS_OPEN,
    CONNECTION_STATUS_WRITE_CLOSED,
    CONNECTION_STATUS_READ_CLOSED
};

struct connection {
    enum connection_status status;
    int fd;
};

struct connectionSet {
    struct connection connections[MAX_CONNECTIONS];
    u_int8_t next_connection;
};

void connectionSet_init(struct connectionSet* set)
{
    set->next_connection = 0;
    for(size_t i = 0; i < MAX_CONNECTIONS; ++i)
        set->connections[i].status = CONNECTION_STATUS_CLOSED;
}

/**
 * returns -1 if no free spot was found
 */
int connectionSet_add_connection(struct connectionSet* set, int fd)
{
    if( set->connections[set->next_connection].status != CONNECTION_STATUS_CLOSED ) {
        char slot_found = 0;
        for( int i = (set->next_connection + 1) % MAX_CONNECTIONS; i != set->next_connection; i = (i + 1) % MAX_CONNECTIONS ) {
            if( set->connections[i].status == CONNECTION_STATUS_CLOSED ) {
                set->next_connection = i;
                slot_found = 1;
                break;
            }
        }
        if( !slot_found ) {
            // close the connection immediately, for we have no connection slot left.
            shutdown(fd, SHUT_RDWR);
            close(fd);
            return -1;
        }
    }

    set->connections[set->next_connection] = (struct connection) {.status = CONNECTION_STATUS_OPEN, .fd = fd };
    int returnValue = set->next_connection;
    set->next_connection = (set->next_connection + 1) % MAX_CONNECTIONS;

    return returnValue;
}

void connectionSet_shutdown_rd(struct connectionSet* set, u_int8_t connection)
{
    struct connection* conn = set->connections + connection;
    switch(conn->status) {
        case CONNECTION_STATUS_OPEN:
            shutdown(conn->fd, SHUT_RD);
            conn->status = CONNECTION_STATUS_READ_CLOSED;
            break;
        case CONNECTION_STATUS_WRITE_CLOSED:
            shutdown(conn->fd, SHUT_RD);
            close(conn->fd);
            conn->status = CONNECTION_STATUS_CLOSED;
            break;
        default:
            assert(0 && "Trying to shut rd an already closed connection");
    }
}

void connectionSet_shutdown_wr(struct connectionSet* set, u_int8_t connection)
{
    struct connection* conn = set->connections + connection;
    switch(conn->status) {
        case CONNECTION_STATUS_OPEN:
            shutdown(conn->fd, SHUT_WR);
            conn->status = CONNECTION_STATUS_WRITE_CLOSED;
            break;
        case CONNECTION_STATUS_READ_CLOSED:
            shutdown(conn->fd, SHUT_WR);
            close(conn->fd);
            conn->status = CONNECTION_STATUS_CLOSED;
            break;
        default:
            assert(0 && "Trying to shut wr an already closed connection");
    }
}
