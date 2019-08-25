#include <stdio>
#include <stdio.h>
#include <stdlib.h>
#include <sys/types.h>
#include <netinet/in.h>
#include <errno.h>
#include <unistd.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <limits.h>
#include <string.h>
#include <vector>

static char is_incoming = 1;
#define LOG_MSG(msg, ...) fprintf(stderr, "%s " msg, is_incoming ? "[in] " : "[out]" ,##__VA_ARGS__ )

int runIncomingSide()
{
    LOG_MSG("starting\n");
    int listen_port = 8001;

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
    
    int listen_fd = setupServerSocket(listen_port);
    if( listen_fd < 0 )
        return 1;

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

int main(int argc, char const *argv[])
{
    runIncomingSide();
    return 0;
}
