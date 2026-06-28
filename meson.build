# meson.build for librtmp2

project('librtmp2', 'c',
  version : '0.1.0',
  default_options : ['c_std=c11', 'warning_level=2'],
  meson_version : '>= 0.50.0'
)

cc = meson.get_compiler('c')

# TLS / RTMPS (OpenSSL). Enabled by default; configure with -Dtls=disabled to
# drop the dependency and build a plaintext-only library.
tls_opt = get_option('tls')
openssl_dep = dependency('openssl', required : tls_opt)
lrtmp2_deps = []
lrtmp2_c_args = []
if openssl_dep.found()
  lrtmp2_deps += openssl_dep
  lrtmp2_c_args += '-DLRTMP2_HAVE_TLS'
endif

# Source files
core_sources = [
  'src/core/alloc.c',
  'src/core/buffer.c',
  'src/core/bytes.c',
  'src/core/net.c',
  'src/core/transport.c',
  'src/core/log.c',
  'src/core/errors.c',
  'src/core/lib.c',
]

handshake_sources = [
  'src/handshake/handshake.c',
]

chunk_sources = [
  'src/chunk/chunk_reader.c',
  'src/chunk/chunk_writer.c',
  'src/chunk/chunk_state.c',
]

message_sources = [
  'src/message/message.c',
  'src/message/control.c',
  'src/message/command.c',
]

amf_sources = [
  'src/amf/amf0.c',
  'src/amf/amf3.c',
  'src/amf/amf_common.c',
]

flv_sources = [
  'src/flv/audio_tag.c',
  'src/flv/video_tag.c',
  'src/flv/script_tag.c',
]

ertmp_sources = [
  'src/ertmp/exvideo.c',
  'src/ertmp/metadata.c',
  'src/ertmp/exaudio.c',
  'src/ertmp/fourcc.c',
  'src/ertmp/connect_caps.c',
  'src/ertmp/reconnect.c',
  'src/ertmp/multitrack.c',
  'src/ertmp/modex.c',
]

session_sources = [
  'src/session/conn.c',
  'src/session/stream.c',
  'src/session/publish.c',
  'src/session/play.c',
  'src/session/state_machine.c',
]

server_sources = [
  'src/server/server.c',
]

client_sources = [
  'src/client/client.c',
]

all_sources = (core_sources + handshake_sources + chunk_sources +
              message_sources + amf_sources + flv_sources + ertmp_sources +
              session_sources + server_sources + client_sources)

# Library
librtmp2_shared = shared_library('librtmp2',
  all_sources,
  include_directories : include_directories('include', 'src'),
  c_args : lrtmp2_c_args,
  dependencies : lrtmp2_deps,
  version : meson.project_version(),
  soversion : '0',
  install : true,
)

librtmp2_static = static_library('librtmp2',
  all_sources,
  include_directories : include_directories('include', 'src'),
  c_args : lrtmp2_c_args,
  dependencies : lrtmp2_deps,
  install : true,
)

# Declare dependency for subprojects / pkg-config
librtmp2_dep = declare_dependency(
  link_with : librtmp2_static,
  include_directories : include_directories('include', 'src'),
  dependencies : lrtmp2_deps,
)

# pkg-config
pkg = import('pkgconfig')
pkg.generate(
  librtmp2_static,
  name : 'librtmp2',
  filebase : 'librtmp2',
  description : 'Modern C library for Legacy RTMP and Enhanced RTMP v1/v2',
  version : meson.project_version(),
)

# Tests (optional)
build_tests = get_option('tests')
if build_tests
  test_sources = [
    'tests/unit/main.c',
    'tests/unit/test_handshake.c',
    'tests/unit/test_chunk.c',
    'tests/unit/test_amf.c',
    'tests/unit/test_buffer.c',
    'tests/unit/test_ertmp.c',
    'tests/unit/test_control.c',
    'tests/unit/test_server.c',
    'tests/unit/test_message.c',
    'tests/unit/test_net.c',
    'tests/unit/test_transport.c',
  ]
  test_exec = executable('test_librtmp2',
    test_sources,
    dependencies : librtmp2_dep,
    c_args : lrtmp2_c_args,
    include_directories : include_directories('src'),
  )
  test('all', test_exec)
endif

# Examples (optional)
build_examples = get_option('examples')
if build_examples
  executable('minimal_server',
    'examples/minimal_server/minimal_server.c',
    dependencies : librtmp2_dep,
  )
  executable('minimal_client',
    'examples/minimal_client/minimal_client.c',
    dependencies : librtmp2_dep,
  )
  executable('dump_frames',
    'examples/dump_frames/dump_frames.c',
    dependencies : librtmp2_dep,
  )
endif
