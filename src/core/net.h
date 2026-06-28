#ifndef LRTMP2_CORE_NET_H
#define LRTMP2_CORE_NET_H

#include <stddef.h>

/* Split a "host:port" authority into separate host and port strings.
 *
 * Accepts:
 *   - "host:port"        -> host, port
 *   - "host"             -> host, def_port
 *   - "[v6addr]:port"    -> v6addr (brackets stripped), port
 *   - "[v6addr]"         -> v6addr, def_port
 *   - "fe80::1" / "::"   -> the whole string as host, def_port
 *                           (a bare, unbracketed literal with >1 colon)
 *   - ":port"            -> "" (empty host = wildcard), port
 *
 * `def_port` is copied into `port` whenever the input carries no port of its
 * own. Returns 0 on success, or -1 if a destination buffer is too small or the
 * bracketed form is malformed (missing ']' or trailing junk after it).
 */
int lrtmp2_split_host_port(const char *input,
                           char *host, size_t host_sz,
                           char *port, size_t port_sz,
                           const char *def_port);

#endif /* LRTMP2_CORE_NET_H */
