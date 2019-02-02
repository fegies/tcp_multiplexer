#include <stdlib.h>
#include <stdio.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <poll.h>
#include <errno.h>
#include <limits.h>
#include <unistd.h>
#include <fcntl.h>
#include <netinet/in.h>

#define MAX_CONNECTIONS 256
#define MAX_SOCKETS (MAX_CONNECTIONS + 2)
#define HEADER_SIZE 3
#define MAX_PAYLOAD_SIZE (PIPE_BUF - HEADER_SIZE)

int setupServerSocket(int listen_port)
{
    int listen_fd = socket(AF_INET, SOCK_STREAM, 0);

    int output_fd = 0;

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
        fprintf(stderr, "failed to open bind socket to port %d\n", listen_port);
        return -1;
    }
    if( listen(listen_fd, 5) < 0 ) {
        fprintf(stderr, "failed to listen to socket with errno %d\n", errno);
        return -1;
    }
    return listen_fd;
}

enum connection_status {
    CONNECTION_STATUS_CLOSED = 0,
    CONNECTION_STATUS_OPEN,
    CONNECTION_STATUS_SENDER_CLOSED,
    CONNECTION_STATUS_RECEIVER_CLOSED
};

enum fdtype {
    FDTYPE_LISTENSOCKET,
    FDTYPE_CONNECTIONSOCKET,
    FDTYPE_REMOTE_IN
};

struct fdinfo
{
    enum fdtype type;
    uint8_t connection;
};

struct connection {
    enum connection_status status;
    int fd;
};

int accept_child_connection(int listen_fd, struct fdinfo* poll_fdinfo, struct pollfd* poll_fds, unsigned int* poll_fd_count, struct connection* connections, uint8_t* next_connection)
{
    int child_connection = accept(listen_fd, 0, 0);

    if( connections[*next_connection].status != CONNECTION_STATUS_CLOSED) {
        char connection_slot_found = 0;
        for( int i = (*next_connection + 1) % MAX_CONNECTIONS; i != *next_connection; i = (i + 1) % MAX_CONNECTIONS ) {
            if( connections[i].status == CONNECTION_STATUS_CLOSED ) {
                *next_connection = i;
                connection_slot_found = 1;
                break;
            }
        }
        if( !connection_slot_found ) {
            // close the connection immediately, for we have no connection slot left.
        }
    }

    connections[*next_connection] = { CONNECTION_STATUS_OPEN, child_connection };
    ++(*poll_fd_count);
    poll_fds[*poll_fd_count].fd = child_connection;
    poll_fdinfo[*poll_fd_count] = { FDTYPE_CONNECTIONSOCKET, *next_connection };
    *next_connection = (*next_connection + 1 ) % MAX_CONNECTIONS;
    return 0;
}

int main(int argc, const char** argv)
{
    
    unsigned int poll_fd_count = 2;
    int listen_port = 8000;

    int remote_read_fd = 1;
    int listen_fd = setupServerSocket(listen_port);
    if( listen_fd < 0 )
        return 1;

    struct connection connections[MAX_CONNECTIONS] = {0};
    u_int8_t next_connection = 0;

    struct fdinfo poll_fdinfo[MAX_SOCKETS];
    struct pollfd poll_fds[MAX_SOCKETS];
    for(int i = 0; i < MAX_SOCKETS; ++i)
        poll_fds[i].events = POLLIN;
    poll_fds[0].fd = listen_fd;
    poll_fdinfo[0].type = FDTYPE_LISTENSOCKET;
    poll_fds[1].fd = remote_read_fd;
    poll_fdinfo[1].type = FDTYPE_REMOTE_IN;


    //main polling loop
    while(1) {
        int poll_return = poll(poll_fds, poll_fd_count, -1);
        if( poll_return < 0 ) {
            if( errno == EINTR || errno == EAGAIN)
                continue;
            else {
                fprintf(stderr, "Poll failed with errno %d\n", errno);
                return 1;
            }
        }
        for(unsigned char i = 0; poll_return > 0 && i < poll_fd_count; --poll_return,++i) {
            if( poll_fds[i].revents != 0 ) {
                if( (poll_fds[i].revents & POLLIN) == POLLIN ) {
                    switch(poll_fdinfo[i].type) {
                        case FDTYPE_LISTENSOCKET:
                            accept_child_connection(listen_fd, poll_fdinfo, poll_fds, &poll_fd_count, connections, &next_connection);
                            break;
                        case FDTYPE_CONNECTIONSOCKET:
                            break;
                        case FDTYPE_REMOTE_IN:
                            break;
                    }
                    u_int8_t message_buffer[PIPE_BUF];
                    int read_count = read(poll_fds[i].fd, message_buffer + HEADER_SIZE, MAX_PAYLOAD_SIZE);
                    if( read_count < 0 ) {
                        fprintf(stderr, "READ ERROR\n");
                        return 2;
                    }
                    else {
                        u_int8_t connection = i-1;
                        u_int16_t readsize = read_count;
                        message_buffer[0] = connection;
                        message_buffer[1] = (u_int8_t) (readsize >> 8);
                        message_buffer[2] = (u_int8_t) (readsize);
                        write(output_fd, message_buffer, HEADER_SIZE + read_count);
                    }
                }
                else {
                    fprintf(stderr, "ON INPUT POLL\n");
                    return 4;
                }
            }
        }
    }

    close(listen_fd);
    return 0;
}
