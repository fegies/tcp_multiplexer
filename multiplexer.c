#include <stdio.h>
#include <sys/types.h>
#include <netinet/in.h>
#include <errno.h>
#include <unistd.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <limits.h>
#include <string.h>

#include "constants.h"
#include "pollSet.h"
#include "connectionSet.h"

static char is_incoming = 1;

#define LOG_MSG(msg, ...) fprintf(stderr, "%s " msg, is_incoming ? "[incoming]" : "[outgoing]", ##__VA_ARGS__ )

int setupServerSocket(int listen_port)
{
    int listen_fd = socket(AF_INET, SOCK_STREAM, 0);

    if(listen_fd == -1) {
        fprintf(stderr, "could not open socket..\n");
        return 1;
    }

    struct sockaddr_in addr;
    addr.sin_port = htons(listen_port);
    addr.sin_addr.s_addr = 0;
    addr.sin_addr.s_addr = INADDR_ANY;
    addr.sin_family = AF_INET;
    if(bind(listen_fd, (struct sockaddr*)&addr, sizeof(struct sockaddr_in)) == -1) {
        fprintf(stderr, "failed to bind socket to port %d\n", listen_port);
        return -1;
    }
    if( listen(listen_fd, 5) < 0 ) {
        fprintf(stderr, "failed to listen to socket with errno %d\n", errno);
        return -1;
    }
    return listen_fd;
}

int openOutgoingSocket() {
    struct sockaddr_in addr;
    addr.sin_port = htons(8000);
    inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);
    addr.sin_family = AF_INET;

    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if( fd < 0 )    
        return -1;
    if( connect(fd, (struct sockaddr*) &addr, sizeof(addr)) < 0 )
        return -2;
    return fd;
}

int handleIncomingConnection(int listen_fd, struct pollSet* pollset, struct connectionSet* connectionset)
{
    int child_connection = accept(listen_fd, 0, 0);

    int connection = connectionSet_add_connection(connectionset, child_connection);
    if( connection < 0 )
        return connection;

    assert(
        pollSet_add_socket(pollset, child_connection, (struct fdinfo) {.type = FDTYPE_CONNECTIONSOCKET, .connection = connection})
        >= 0
    );

    LOG_MSG("accepting new connection %d\n", connection);

    return 0;
}

int handleIncomingRead(struct pollSet* pollset, size_t index, struct connectionSet* connset, int tunnel_fd)
{
    u_int8_t message_buffer[PIPE_BUF];

    int fd = pollset->poll_fds[index].fd;
    int read_count = read(fd, message_buffer + HEADER_SIZE, MAX_PAYLOAD_SIZE);
    if( read_count < 0 ) {
        fprintf(stderr, "READ ERROR\n");
        return -2;
    }
    else {
        u_int8_t connection = pollset->fdinfo[index].connection;
        u_int16_t readsize = read_count;
        message_buffer[0] = connection;
        message_buffer[1] = (u_int8_t) (readsize >> 8);
        message_buffer[2] = (u_int8_t) (readsize);
        LOG_MSG("putting payload of size %d into tunnel for connection %d\n", readsize, connection);
        write(tunnel_fd, message_buffer, HEADER_SIZE + read_count);
        if( read_count == 0 ) { // mark the connection as not readable on our side
            LOG_MSG("empty read; closing connection\n");
            connectionSet_shutdown_rd(connset, connection);
            pollSet_remove_socket(pollset, index);
        }
        return 0;
    }
}

int handleIncomingTunnelRead(struct pollSet* pollset, size_t index, struct connectionSet* connset, int tunnel_fd, char open_missing_connections)
{
    u_int8_t header[3];
    read(tunnel_fd, header, 3);
    u_int8_t connection = header[0];
    u_int16_t payload_size = (((u_int16_t) header[1]) << 8) | (u_int16_t) header[2];
    enum connection_status status = connset->connections[connection].status;
    if( status == CONNECTION_STATUS_CLOSED && open_missing_connections ) {
        LOG_MSG("Opening new connection %d to target\n", connection);
        int fd = openOutgoingSocket();
        if( fd < 0 )
            return -1;
        connset->connections[connection] = (struct connection) {.status = CONNECTION_STATUS_OPEN, .fd = fd};
        status = CONNECTION_STATUS_OPEN;
        pollSet_add_socket(pollset, fd, (struct fdinfo) {.type = FDTYPE_CONNECTIONSOCKET, .connection = connection});
    }
    assert(status == CONNECTION_STATUS_OPEN || status == CONNECTION_STATUS_READ_CLOSED);
    if( payload_size == 0 ) { // close the writing end of the connection
        LOG_MSG("closing connection %d\n", connection);
        connectionSet_shutdown_wr(connset, connection);
        pollSet_remove_socket(pollset, index);
    }
    else {
        LOG_MSG("receiving payload of size %d from tunnel for connection %d\n", payload_size, connection);
        u_int8_t message_buffer[payload_size];
        size_t payload_size_received = 0;
        while( payload_size_received < payload_size ) {
            size_t read_size = read(tunnel_fd, message_buffer+payload_size_received, payload_size - payload_size_received);
            assert( read_size >= 0 );
            payload_size_received += read_size;
        }
        LOG_MSG("forwarding payload to connection %d\n", connection);
        write(connset->connections[connection].fd, message_buffer, payload_size);
    }
    return 0;
}

struct callback_arg {
    struct connectionSet* connset;
    int tunnel_fd_rd;
    int tunnel_fd_wr;
};

int poll_incoming_side_callback(struct pollSet *set, size_t index, struct callback_arg* arg)
{
    struct pollfd *pollfd = set->poll_fds + index;
    if( (pollfd->revents & POLLIN) == POLLIN ) {
        struct fdinfo *fdinfo = set->fdinfo + index;
        switch(fdinfo->type) {
            case FDTYPE_LISTENSOCKET:
                handleIncomingConnection(pollfd->fd, set, arg->connset);
                break;
            case FDTYPE_CONNECTIONSOCKET:
                handleIncomingRead(set, index, arg->connset, arg->tunnel_fd_wr);
                break;
            case FDTYPE_TUNNELSOCKET:
                handleIncomingTunnelRead(set, index, arg->connset, arg->tunnel_fd_rd, 0);
                break;
        }
        return 0;
    }
    else {
        fprintf(stderr, "ON INPUT POLL\n");
        return -1;
    }
}
int poll_incoming_side_callback_vp(struct pollSet *set, size_t index, void* arg)
{
    return poll_incoming_side_callback(set, index, arg);
}

int poll_outgoing_side_callback(struct pollSet *set, size_t index, struct callback_arg *arg)
{
    struct pollfd *pollfd = set->poll_fds + index;
    if( (pollfd->revents & POLLIN) == POLLIN ) {
        struct fdinfo *fdinfo = set->fdinfo + index;
        switch(fdinfo->type) {
            case FDTYPE_CONNECTIONSOCKET:
                handleIncomingRead(set, index, arg->connset, arg->tunnel_fd_wr);
                break;
            case FDTYPE_TUNNELSOCKET:
                handleIncomingTunnelRead(set, index, arg->connset, arg->tunnel_fd_rd, 1);
                break;
            default:
                assert(0 && "invalid fd type: listensocket on outgoing side");
        }
        return 0;
    }
    else {
        fprintf(stderr, "ON INPUT POLL\n");
        return -1;
    }
}

int poll_outgoing_side_callback_vp(struct pollSet *set, size_t index, void* arg)
{
    return poll_outgoing_side_callback(set, index, arg);
}

int runOutgoingSide()
{
    is_incoming = 0;
    LOG_MSG("starting\n");
    int tunnel_fd_rd = 0;
    int tunnel_fd_wr = 1;

    struct connectionSet connset;
    connectionSet_init(&connset);

    struct pollSet pollset;
    pollSet_init(&pollset);
    pollSet_add_socket(&pollset, tunnel_fd_rd, (struct fdinfo) {.type = FDTYPE_TUNNELSOCKET});

    struct callback_arg callback_arg = (struct callback_arg) {.connset = &connset, .tunnel_fd_rd = tunnel_fd_rd, .tunnel_fd_wr = tunnel_fd_wr};

    char runLoop = 1;
    while(runLoop) {
        LOG_MSG("entering poll state\n");
        if( pollSet_run_poll(&pollset, poll_outgoing_side_callback_vp, &callback_arg) < 0 )
            runLoop = 0;
    }

    return 0;
}

int runIncomingSide()
{
    LOG_MSG("starting\n");
    int listen_port = 8001;

    int listen_fd = setupServerSocket(listen_port);
    if( listen_fd < 0 )
        return 1;

    int out_pipe[2];
    if( pipe(out_pipe) < 0 )
        return 3;
    int in_pipe[2];
    if( pipe(in_pipe) < 0 )
        return 3;

    int child_pid = fork();
    if( child_pid < 0 )
        return 4;
    if( child_pid == 0 ) { //child
        dup2(out_pipe[0], 0);
        close(out_pipe[1]);
        close(in_pipe[0]);
        dup2(in_pipe[1], 1);
        return runOutgoingSide();
    }
    else {
        close(out_pipe[0]);
        close(in_pipe[1]);
    }

    int tunnel_fd_rd = in_pipe[0];
    int tunnel_fd_wr = out_pipe[1];
    

    struct connectionSet connectionset;
    connectionSet_init(&connectionset);
    // struct connection connections[MAX_CONNECTIONS] = {0};

    struct pollSet pollset;
    pollSet_init(&pollset);
    pollSet_add_socket(&pollset, listen_fd, (struct fdinfo) {.type = FDTYPE_LISTENSOCKET});
    pollSet_add_socket(&pollset, tunnel_fd_rd, (struct fdinfo) {.type = FDTYPE_TUNNELSOCKET});

    struct callback_arg callback_arg = (struct callback_arg) { .connset = &connectionset, .tunnel_fd_rd = tunnel_fd_rd, .tunnel_fd_wr = tunnel_fd_wr };

    //main polling loop
    char runLoop = 1;
    while(runLoop) {
        LOG_MSG("entering poll state\n");
        if( pollSet_run_poll(&pollset, poll_incoming_side_callback_vp, &callback_arg) < 0 )
            runLoop = 0;
    }

    close(listen_fd);
    return 0;
}

int main(int argc, const char** argv)
{
    if( argc > 1 && strcmp(argv[1],"--outgoing") == 0 )
        return runOutgoingSide();
    else
        return runIncomingSide();
}
