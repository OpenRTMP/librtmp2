# librtmp2 Makefile

CC ?= gcc
AR ?= ar
CFLAGS_EXTRA ?=

CFLAGS = -Wall -Wextra -Wpedantic -Wshadow -Wstrict-prototypes -Wno-implicit-function-declaration -Wno-return-type
CFLAGS += -Iinclude -Isrc

ifdef DEBUG
  CFLAGS += -g -O0 -DDEBUG
else
  CFLAGS += -O2 -DNDEBUG
endif

ifdef ASAN
  CFLAGS += -fsanitize=address -fno-omit-frame-pointer
  LDFLAGS += -fsanitize=address
endif

ifdef UBSAN
  CFLAGS += -fsanitize=undefined
  LDFLAGS += -fsanitize=undefined
endif

SRCS := $(wildcard src/core/*.c src/handshake/*.c src/chunk/*.c src/message/*.c \
          src/amf/*.c src/flv/*.c src/ertmp/*.c src/session/*.c src/server/*.c src/client/*.c)
OBJS := $(SRCS:.c=.o)
LIB_SO = liblibrtmp2.so
LIB_A  = liblibrtmp2.a
TEST_BIN = tests/run_tests

TARGETS = $(LIB_SO) $(LIB_A)

# Default target
.PHONY: debug release test asan fuzz install clean

debug:
	$(MAKE) DEBUG=1 all

release:
	$(MAKE) all

asan:
	$(MAKE) DEBUG=1 ASAN=1 all

ubsan:
	$(MAKE) DEBUG=1 UBSAN=1 all

fuzz: asan
	@echo "Fuzz targets built with ASan"
	@echo "Run with: clang -fsanitize=fuzzer,address -o fuzz_all fuzz_entry.c [objects]"

all: $(TARGETS) $(TEST_BIN)

$(LIB_SO): $(OBJS)
	$(CC) -shared -o $@ $(OBJS) $(LDFLAGS)

$(LIB_A): $(OBJS)
	$(AR) rcs $@ $^

src/%.o: src/%.c
	$(CC) $(CFLAGS) -fPIC -c -o $@ $<

src/%/%.o: src/%/%.c
	$(CC) $(CFLAGS) -fPIC -c -o $@ $<

# Test binary
TEST_SRCS := $(wildcard tests/unit/*.c)
TEST_OBJS := $(TEST_SRCS:.c=.o)

$(TEST_BIN): $(OBJS) $(TEST_OBJS)
	$(CC) $(CFLAGS) -o $@ $(OBJS) $(TEST_OBJS) $(LDFLAGS) -lm

tests/%.o: tests/%.c
	$(CC) $(CFLAGS) -fPIC -c -o $@ $<

# Examples
examples/minimal_server/minimal_server.c: $(LIB_A)

# Install
PREFIX ?= /usr/local

install: $(LIB_SO) $(LIB_A)
	install -d $(PREFIX)/lib $(PREFIX)/include/librtmp2 $(PREFIX)/lib/pkgconfig
	install -m 755 $(LIB_SO) $(PREFIX)/lib/$(LIB_SO).0.1.0
	ln -sf $(LIB_SO).0.1.0 $(PREFIX)/lib/$(LIB_SO).0
	ln -sf $(LIB_SO).0 $(PREFIX)/lib/$(LIB_SO)
	install -m 644 $(LIB_A) $(PREFIX)/lib/
	install -m 644 include/librtmp2/*.h $(PREFIX)/include/librtmp2/
	sed 's|@PREFIX@|$(PREFIX)|g; s|@LIBS@|-llibrtmp2|g' librtmp2.pc.in > $(PREFIX)/lib/pkgconfig/librtmp2.pc

test: $(TEST_BIN)
	./$(TEST_BIN)

INTEGRATION_BIN = tests/integration/run_ingest
CLIENT_INTEGRATION_BIN = tests/integration/run_client
ERTMP_INTEGRATION_BIN = tests/integration/run_ertmp_v1
ERTMP_V2_INTEGRATION_BIN = tests/integration/run_ertmp_v2

$(INTEGRATION_BIN): $(OBJS) tests/integration/test_server_ingest.c
	$(CC) $(CFLAGS) -o $@ tests/integration/test_server_ingest.c $(OBJS) $(LDFLAGS) -lm -lpthread

$(CLIENT_INTEGRATION_BIN): $(OBJS) tests/integration/test_client_publish.c
	$(CC) $(CFLAGS) -o $@ tests/integration/test_client_publish.c $(OBJS) $(LDFLAGS) -lm -lpthread

$(ERTMP_INTEGRATION_BIN): $(OBJS) tests/integration/test_server_ertmp_v1.c
	$(CC) $(CFLAGS) -o $@ tests/integration/test_server_ertmp_v1.c $(OBJS) $(LDFLAGS) -lm -lpthread

$(ERTMP_V2_INTEGRATION_BIN): $(OBJS) tests/integration/test_server_ertmp_v2.c
	$(CC) $(CFLAGS) -o $@ tests/integration/test_server_ertmp_v2.c $(OBJS) $(LDFLAGS) -lm -lpthread

clean:
	rm -f $(OBJS) $(TEST_OBJS) $(TARGETS) $(TEST_BIN) $(INTEGRATION_BIN) $(CLIENT_INTEGRATION_BIN) $(ERTMP_INTEGRATION_BIN) $(ERTMP_V2_INTEGRATION_BIN) examples/**/*.o tests/integration/*.o
