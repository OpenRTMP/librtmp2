/**
 * net.c — small networking helpers shared by the server (bind) and client
 * (connect) entry points.
 */
#include "core/net.h"
#include <string.h>
#include <stdio.h>

int lrtmp2_split_host_port(const char *input,
                           char *host, size_t host_sz,
                           char *port, size_t port_sz,
                           const char *def_port)
{
    if (!input || !host || !port || host_sz == 0 || port_sz == 0) return -1;

    snprintf(port, port_sz, "%s", def_port ? def_port : "");

    /* Bracketed IPv6 literal: "[addr]" or "[addr]:port". */
    if (input[0] == '[') {
        const char *end = strchr(input, ']');
        if (!end) return -1;
        size_t hlen = (size_t)(end - (input + 1));
        if (hlen >= host_sz) return -1;
        memcpy(host, input + 1, hlen);
        host[hlen] = '\0';
        if (end[1] == ':') {
            snprintf(port, port_sz, "%s", end + 2);
        } else if (end[1] != '\0') {
            return -1;  /* junk after the closing bracket */
        }
        return 0;
    }

    /* Count colons to tell "host:port" apart from a bare IPv6 literal. */
    size_t colons = 0;
    const char *last_colon = NULL;
    for (const char *p = input; *p; p++) {
        if (*p == ':') { colons++; last_colon = p; }
    }

    if (colons > 1) {
        /* Unbracketed and multi-colon -> a bare IPv6 literal with no port. */
        if (strlen(input) >= host_sz) return -1;
        snprintf(host, host_sz, "%s", input);
        return 0;
    }

    if (colons == 1) {
        size_t hlen = (size_t)(last_colon - input);
        if (hlen >= host_sz) return -1;
        memcpy(host, input, hlen);
        host[hlen] = '\0';
        snprintf(port, port_sz, "%s", last_colon + 1);
        return 0;
    }

    /* No colon: host only, default port. */
    if (strlen(input) >= host_sz) return -1;
    snprintf(host, host_sz, "%s", input);
    return 0;
}
