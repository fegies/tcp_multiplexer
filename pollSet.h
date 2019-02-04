#pragma once

#include <poll.h>
#include <errno.h>
#include <sys/types.h>
#include <assert.h>
#include <stdio.h>

#include "constants.h"


enum fdtype {
    FDTYPE_LISTENSOCKET,
    FDTYPE_CONNECTIONSOCKET,
    FDTYPE_TUNNELSOCKET
};


struct fdinfo
{
    enum fdtype type;
    connection_t connection;
};

struct pollSet {
    struct pollfd poll_fds[MAX_SOCKETS];
    struct fdinfo fdinfo[MAX_SOCKETS];
    char dirty;
    size_t length; //number of used sockets
};

void pollSet_init(struct pollSet* set)
{
    for(int i = 0; i < MAX_SOCKETS; ++i)
        set->poll_fds[i].events = POLLIN;
    set->dirty = 0;
    set->length = 0;
}

/**
 * returns -1 if there is no space to add the socket
 * does not modify the socket itself
 */
int pollSet_add_socket(struct pollSet* set, int fd, struct fdinfo info)
{
    if( set->length == MAX_SOCKETS ) {
        return -1;
    }
    set->dirty = 1;
    set->poll_fds[set->length].fd = fd;
    set->fdinfo[set->length] = info;
    set->length++;
    return 0;
}

/**
 * returns -1 if an error occurred while removing the socket
 */
int pollSet_remove_socket(struct pollSet* set, size_t index)
{
    assert(set->length > 0);
    if( index >= set->length)
        return 0;
    
    set->dirty = 1;
    --set->length;
    set->poll_fds[index] = set->poll_fds[set->length];
    set->fdinfo[index] = set->fdinfo[set->length];
    return 0;
}

/**
 * runs poll and calls the callback on all entries that have nonzero revent fields
 * returns -1 if the poll failed, 0 otherwise
 * also passes through the exit code of the callback if it is < 0
 * 
 * passes through the arg to the callback unmodified
 */
int pollSet_run_poll(struct pollSet* set, int (*callback)(struct pollSet* set, size_t index, void* arg), void* arg)
{
    char retryPoll;
    do {
        retryPoll = 0;
        int poll_return = poll(set->poll_fds, set->length, -1);

        if( poll_return < 0 ) {
            if( errno == EINTR || errno == EAGAIN) {
                retryPoll = 1;
                continue;
            }
            else {
                fprintf(stderr, "Poll failed with errno %d\n", errno);
                return -1;
            }
        }

        char redoCheckLoop;
        do {
            redoCheckLoop = 0;
            set->dirty = 0;
            for(size_t i = 0; poll_return > 0 && i < set->length; ++i) {
                if( set->poll_fds[i].revents != 0 ) {
                    int cb_return = callback(set, i, arg);
                    --poll_return;
                    if( cb_return < 0 )
                        return cb_return;
                    if(set->dirty) {
                        redoCheckLoop = 1;
                        break;
                    }
                }
            }
        }
        while(redoCheckLoop);
    }
    while(retryPoll);
    return 0;
}