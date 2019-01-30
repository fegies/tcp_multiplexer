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
#define MAX_SOCKETS (MAX_CONNECTIONS + 1)
#define HEADER_SIZE 3
#define MAX_PAYLOAD_SIZE (PIPE_BUF - HEADER_SIZE)

int main(int argc, const char** argv)
{
    int listen_port = 8000;

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
        return 1;
    }
    if( listen(listen_fd, 5) < 0 ) {
        fprintf(stderr, "failed to listen to socket with errno %d\n", errno);
        return 1;
    }
    unsigned int sockets_open = 1;

    struct pollfd poll_fds[MAX_SOCKETS];
    for(int i = 0; i < MAX_SOCKETS; ++i)
        poll_fds[i].events = POLLIN;
    poll_fds[0].fd = listen_fd;

    while(1) {
        int poll_return = poll(poll_fds, sockets_open, -1);
        if( poll_return < 0 ) {
            if( errno == EINTR )
                continue;
            else {
                fprintf(stderr, "Poll failed with errno %d\n", errno);
                return 1;
            }
        }
        if( poll_fds[0].revents != 0 ) { // accept a connection socket
            if( (poll_fds[0].revents & POLLIN) == POLLIN ) {
                int child_fd = accept(listen_fd, 0, 0);
                poll_fds[sockets_open++].fd = child_fd;
                --poll_return;
            }
            else {
                fprintf(stderr, "POLL ERROR \n");
                return 1;
            }
        }
        for(unsigned char i = 1; poll_return > 0 && i < sockets_open; --poll_return,++i) {
            if( poll_fds[i].revents != 0 ) {
                if( (poll_fds[i].revents & POLLIN) == POLLIN ) {
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
            }
            else {
                fprintf(stderr, "ON INPUT POLL\n");
                return 4;
            }
        }
    }

    close(listen_fd);
    return 0;
}
