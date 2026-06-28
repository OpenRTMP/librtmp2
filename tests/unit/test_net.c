/**
 * test_net.c — Unit tests for host:port parsing (IPv4, IPv6, hostnames)
 */
#include "core/net.h"
#include <stdio.h>
#include <string.h>

/* Expect a successful split with the given host/port. */
static int expect_ok(const char *in, const char *want_host, const char *want_port)
{
    char host[256], port[16];
    int rc = lrtmp2_split_host_port(in, host, sizeof(host), port, sizeof(port), "1935");
    if (rc != 0) {
        printf("FAIL: split(\"%s\") returned %d, expected 0\n", in, rc);
        return 0;
    }
    if (strcmp(host, want_host) != 0 || strcmp(port, want_port) != 0) {
        printf("FAIL: split(\"%s\") = host=\"%s\" port=\"%s\", expected host=\"%s\" port=\"%s\"\n",
               in, host, port, want_host, want_port);
        return 0;
    }
    return 1;
}

/* Expect the input to be rejected. */
static int expect_err(const char *in)
{
    char host[256], port[16];
    int rc = lrtmp2_split_host_port(in, host, sizeof(host), port, sizeof(port), "1935");
    if (rc == 0) {
        printf("FAIL: split(\"%s\") accepted, expected rejection\n", in);
        return 0;
    }
    return 1;
}

int test_net_main(void)
{
    int passed = 0;
    int total = 0;
    printf("Running net (host:port parse) tests...\n");

    /* IPv4 / hostnames */
    total++; passed += expect_ok("1.2.3.4:1935", "1.2.3.4", "1935");
    total++; passed += expect_ok("0.0.0.0:1935", "0.0.0.0", "1935");
    total++; passed += expect_ok("example.com:8080", "example.com", "8080");
    total++; passed += expect_ok("example.com", "example.com", "1935");   /* default port */
    total++; passed += expect_ok("127.0.0.1", "127.0.0.1", "1935");

    /* Bracketed IPv6 */
    total++; passed += expect_ok("[::1]:1935", "::1", "1935");
    total++; passed += expect_ok("[2001:db8::1]:5000", "2001:db8::1", "5000");
    total++; passed += expect_ok("[::1]", "::1", "1935");                  /* default port */

    /* Bare (unbracketed) IPv6 literal -> no port */
    total++; passed += expect_ok("::1", "::1", "1935");
    total++; passed += expect_ok("fe80::1", "fe80::1", "1935");

    /* Wildcard: empty host */
    total++; passed += expect_ok(":1935", "", "1935");

    /* Malformed */
    total++; passed += expect_err("[::1");        /* missing closing bracket */
    total++; passed += expect_err("[::1]junk");   /* trailing junk after bracket */

    if (passed == total) {
        printf("PASS: all %d host:port cases\n", total);
    }
    printf("Net tests: %d/%d passed\n", passed, total);
    return (passed == total) ? 0 : 1;
}
