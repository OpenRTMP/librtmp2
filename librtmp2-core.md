# librtmp2 Core — Projektkonzept

## Zweck

`librtmp2` ist eine moderne Open-Source-C-Bibliothek für **Legacy RTMP** und **Enhanced RTMP v1/v2**, gedacht als wiederverwendbare Protokollbasis für eigene Server, Clients, Relay-Software, OBS-/FFmpeg-Integrationen und spätere eigene Produkte. Die Bibliothek ist bewusst **nicht** selbst ein Mediaserver, sondern die unterste Schicht: Handshake, Chunking, AMF, Commands, Audio/Video-Tags, E-RTMP-Erweiterungen, Zustandsmaschine und Callback-API. [1][2]

Das Projekt soll unter dem GitHub-Account **`AlexanderWagnerDev`** veröffentlicht werden, mit einem Ziel-Repository wie `AlexanderWagnerDev/librtmp2`. [3]

***

## Hauptziele

- Vollständige **Legacy-RTMP**-Basis implementieren: Handshake, Chunk Streams, Message Reassembly, Commands, Control Messages. [2]
- **E-RTMP v1** unterstützen: ExVideoTagHeader, FourCC-Codecs, Metadata-Erweiterungen, HDR-Felder, Audio-Erweiterungen. [4]
- **E-RTMP v2** unterstützen: Capability-Negotiation, `videoFourCcInfoMap`, Reconnect-Mechanismus, Multitrack, ModEx. [5][6]
- Eine **saubere C-API** bereitstellen, damit spätere Server oder Tools in C, C++, Rust, Go, Python, PHP oder anderen FFI-fähigen Sprachen darauf aufbauen können.
- Strikte **Trennung zwischen Core und Produktlogik**: keine HTTP-API, keine Stats-Seite, keine Datenbank, keine Auth-Policy im Core.

***

## Nicht-Ziele

Diese Dinge gehören **nicht** in `librtmp2`:

- Kein HTTP-Server
- Keine Web-UI
- Keine Stats-Webseite
- Keine REST-API
- Keine Persistenz / Datenbank
- Keine Docker-spezifische Produktlogik
- Keine Push-Targets zu Drittplattformen
- Kein FFmpeg-Wrapper
- Kein vollständiger Mediaserver mit Business-Logik

`librtmp2` ist absichtlich die **Protokollbibliothek**, nicht das fertige Produkt.

***

## Architekturprinzipien

### 1. C als Kernsprache

Die Core-Library soll in **C** geschrieben werden, weil sie später möglichst breit einsetzbar sein soll. Das macht direkte Nutzung in OBS, FFmpeg, GStreamer, nginx-Module, Rust-FFI, Go-Cgo oder Python-FFI möglich. Eine native C-Library ist für Infrastrukturprojekte die universellste Form der Distribution.

### 2. Kleine, stabile ABI

Die öffentliche API soll klein, versionierbar und langfristig stabil gehalten werden. Interne Strukturen dürfen sich ändern; öffentliche Header müssen möglichst selten brechen.

### 3. Strict Core / Thin Host

Die Library verarbeitet Bytes, Frames, Commands und Protokollzustände. Was ein Hostprogramm damit macht, entscheidet die Hostanwendung über Callbacks und Konfigurationsstrukturen.

### 4. Deterministische Parser

Alle Parser müssen deterministisch, bounds-checked und fuzzbar sein. Kein undefined behavior, keine impliziten Annahmen über eingehende Pakete.

### 5. Graceful Degradation

Unbekannte E-RTMP-v2-Erweiterungen wie unbekannte ModEx-Typen oder unbekannte Capability-Felder dürfen **nicht** sofort hart abbrechen, sondern müssen protokollgerecht ignoriert oder als „unsupported“ markiert werden. [5][6]

***

## Repository-Struktur

```text
librtmp2/
├── README.md
├── LICENSE
├── CHANGELOG.md
├── CONTRIBUTING.md
├── Makefile
├── meson.build
├── include/
│   └── librtmp2/
│       ├── librtmp2.h
│       ├── version.h
│       ├── types.h
│       ├── errors.h
│       ├── callbacks.h
│       ├── server.h
│       ├── client.h
│       ├── frame.h
│       ├── audio.h
│       ├── video.h
│       ├── amf.h
│       └── ertmp.h
├── src/
│   ├── core/
│   │   ├── alloc.c
│   │   ├── buffer.c
│   │   ├── bytes.c
│   │   ├── log.c
│   │   └── errors.c
│   ├── handshake/
│   │   ├── handshake.c
│   │   └── handshake.h
│   ├── chunk/
│   │   ├── chunk_reader.c
│   │   ├── chunk_writer.c
│   │   ├── chunk_state.c
│   │   └── chunk_internal.h
│   ├── message/
│   │   ├── message.c
│   │   ├── control.c
│   │   ├── command.c
│   │   └── user_control.c
│   ├── amf/
│   │   ├── amf0.c
│   │   ├── amf3.c
│   │   ├── amf_common.c
│   │   └── amf_internal.h
│   ├── flv/
│   │   ├── audio_tag.c
│   │   ├── video_tag.c
│   │   └── script_tag.c
│   ├── ertmp/
│   │   ├── exvideo.c
│   │   ├── fourcc.c
│   │   ├── metadata.c
│   │   ├── connect_caps.c
│   │   ├── reconnect.c
│   │   ├── multitrack.c
│   │   └── modex.c
│   ├── session/
│   │   ├── conn.c
│   │   ├── stream.c
│   │   ├── publish.c
│   │   ├── play.c
│   │   └── state_machine.c
│   ├── server/
│   │   ├── server.c
│   │   └── server_internal.h
│   └── client/
│       ├── client.c
│       └── client_internal.h
├── tests/
│   ├── unit/
│   ├── integration/
│   ├── fuzz/
│   └── fixtures/
├── examples/
│   ├── minimal_server/
│   ├── minimal_client/
│   └── dump_frames/
├── docs/
│   ├── architecture.md
│   ├── protocol-mapping-legacy.md
│   ├── protocol-mapping-ertmp-v1.md
│   ├── protocol-mapping-ertmp-v2.md
│   ├── abi-policy.md
│   └── roadmap.md
└── .github/
    └── workflows/
        ├── ci.yml
        ├── interop.yml
        └── release.yml
```

***

## Öffentliche API-Idee

Die API muss niedrigstufig genug sein, um flexibel zu bleiben, aber hoch genug, damit nicht jeder Host Chunk-Reassembly selbst machen muss.

### Zentrale Typen

```c
typedef struct lrtmp2_server lrtmp2_server_t;
typedef struct lrtmp2_client lrtmp2_client_t;
typedef struct lrtmp2_conn lrtmp2_conn_t;
typedef struct lrtmp2_stream lrtmp2_stream_t;
typedef struct lrtmp2_frame lrtmp2_frame_t;
typedef struct lrtmp2_error lrtmp2_error_t;
```

### Zentrale Konstruktoren

```c
lrtmp2_server_t *lrtmp2_server_create(const lrtmp2_server_config_t *config);
void lrtmp2_server_destroy(lrtmp2_server_t *server);

int lrtmp2_server_listen(lrtmp2_server_t *server, const char *bind_addr);
int lrtmp2_server_poll(lrtmp2_server_t *server, int timeout_ms);

lrtmp2_client_t *lrtmp2_client_create(const lrtmp2_client_config_t *config);
void lrtmp2_client_destroy(lrtmp2_client_t *client);
int lrtmp2_client_connect(lrtmp2_client_t *client, const char *url);
```

### Callback-Modell

```c
typedef int (*lrtmp2_on_connect_cb)(lrtmp2_conn_t *conn, void *userdata);
typedef int (*lrtmp2_on_publish_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int (*lrtmp2_on_play_cb)(lrtmp2_conn_t *conn, const char *app, const char *stream_key, void *userdata);
typedef int (*lrtmp2_on_frame_cb)(lrtmp2_conn_t *conn, const lrtmp2_frame_t *frame, void *userdata);
typedef void (*lrtmp2_on_close_cb)(lrtmp2_conn_t *conn, void *userdata);
```

Der Host registriert diese Hooks und entscheidet, ob ein Publish erlaubt wird, wo Frames hingeleitet werden oder wie Logging und Auth funktionieren.

***

## Protokollmodule

### 1. Handshake-Modul

Legacy RTMP nutzt den klassischen C0/C1/C2 ↔ S0/S1/S2-Handshake. Die Library muss mindestens den Standard-Handshake vollständig unterstützen; komplexere Adobe-Varianten können später ergänzt werden. [2]

Aufgaben:
- Version erkennen
- Fixed-Length-Handshake robust lesen/schreiben
- Timeouts behandeln
- Partial Reads korrekt puffern

### 2. Chunk-Modul

RTMP fragmentiert Nachrichten in Chunks mit Basic Header, Message Header, optional Extended Timestamp und Payload. Die Library braucht vollständige Reassembly pro Chunk Stream ID, inklusive Header-Typen 0–3 und State-Carry-Forward. [2]

Aufgaben:
- Chunk Reader
- Chunk Writer
- State je `csid`
- `SetChunkSize` sofort anwenden
- `Abort` korrekt behandeln

### 3. Message-Modul

Message-Reassembly erzeugt semantische Nachrichten wie `SetChunkSize`, `Acknowledgement`, `WindowAcknowledgementSize`, `UserControlMessage`, Audio-, Video- und Command-Nachrichten. [2]

### 4. AMF-Modul

AMF0 ist Pflicht, AMF3 optional aber sinnvoll für Vollständigkeit. Connect-, CreateStream-, Publish- und Play-Flows benötigen sauberes Encoding und Decoding verschachtelter Objekte, Arrays und Strings.

### 5. Session- und Command-Modul

Die Bibliothek braucht eine interne Zustandsmaschine für:
- `connect`
- `createStream`
- `publish`
- `play`
- `deleteStream`
- `FCPublish`
- `FCUnpublish`

Das Ziel ist, Hosts nicht mit rohen AMF-Arrays alleine zu lassen.

***

## Enhanced RTMP v1

E-RTMP v1 erweitert RTMP/FLV vor allem um moderne Codecs, FourCC-Signalisierung und Metadaten. [4]

### Kernpunkte

- Erkennen des `IsExHeader`-Bits im VideoTagHeader
- Wechsel von Legacy `CodecID` zu `PacketType + FourCC`
- Unterstützung für FourCC-basierte Codecs wie `hvc1`, `av01`, `vp09` [4]
- Erweiterte `PacketTypeMetadata`-Frames für Dinge wie `colorInfo` und HDR-Metadaten [4]
- `fourCcList` im Connect-Objekt unterstützen [4]

### Interne Strukturen

```c
typedef struct {
    uint8_t is_ex_header;
    uint8_t packet_type;
    char fourcc[5];
    uint8_t frame_type;
    uint32_t composition_time;
} lrtmp2_video_header_t;
```

***

## Enhanced RTMP v2

E-RTMP v2 ergänzt laut Spezifikation insbesondere Capability-Negotiation, Multitrack, Reconnect und ModEx. [5][6]

### Kernpunkte

- `capsEx` und `videoFourCcInfoMap` im Connect-/Response-Austausch [5]
- Reconnect-Mechanismus für kontrollierte Umleitung oder Wartung [6]
- Mehrere Tracks pro Session / Stream [6]
- ModEx als Erweiterungsmechanismus ohne harte Protokollbrüche [5]

### Interne Aufgaben

- Capability-Objekte parsen und serialisieren
- Track-Deskriptoren verwalten
- Reconnect-Frames empfangen und senden
- Unbekannte ModEx-Typen protokollieren und ignorieren

***

## State Machine

```text
TCP_ACCEPTED
  -> HANDSHAKE
  -> CONNECTED
  -> APP_CONNECTED
  -> STREAM_CREATED
  -> PUBLISHING | PLAYING
  -> CLOSING
  -> CLOSED
```

Mit E-RTMP v2 kommt logisch ein zusätzlicher Zustand für Capability-Negotiation dazu:

```text
CONNECTED
  -> CAPS_NEGOTIATED
  -> STREAM_CREATED
```

Diese Zustandsmaschine soll vollständig im Core implementiert werden, damit Hostanwendungen auf semantisch sinnvolle Events reagieren können.

***

## Speicher- und Sicherheitsregeln

- Alle Eingabelängen vor jedem Read / Copy validieren
- Keine direkten `malloc(len)` ohne Upper Bounds
- Kein Vertrauen in Payload-Längen aus dem Netzwerk
- Optional eigener Allocator-Hook für Hostintegration
- Fuzzing für Handshake, Chunk Reader, AMF und ExVideoTagHeader
- CI mit AddressSanitizer und UndefinedBehaviorSanitizer

***

## Fehlerklassen

```c
typedef enum {
    LRTMP2_OK = 0,
    LRTMP2_ERR_IO,
    LRTMP2_ERR_TIMEOUT,
    LRTMP2_ERR_PROTOCOL,
    LRTMP2_ERR_HANDSHAKE,
    LRTMP2_ERR_CHUNK,
    LRTMP2_ERR_AMF,
    LRTMP2_ERR_UNSUPPORTED,
    LRTMP2_ERR_AUTH,
    LRTMP2_ERR_INTERNAL
} lrtmp2_error_code_t;
```

Fehler müssen maschinenlesbar und menschenlesbar verfügbar sein.

***

## Testing-Strategie

### Unit-Tests

- Handshake mit Golden Bytes
- Chunk Typ 0–3
- Extended Timestamp
- AMF0 primitive und komplexe Objekte
- ExVideoTagHeader Parsing
- FourCC Parsing
- Capability-Negotiation Parsing

### Integrationstests

- OBS → `librtmp2` Minimalserver
- FFmpeg → `librtmp2` Minimalserver
- `librtmp2` Minimalclient → SRS
- Später HaishinKit → `librtmp2`

### Fuzzing

- Handshake Parser
- Chunk Reader
- AMF Decoder
- E-RTMP Header Parser

***

## Build-System

Empfehlung:
- **Meson** oder **CMake** für Plattform-Portabilität
- zusätzlich einfacher `Makefile` für Linux-Entwicklung

Beispielziele:
- `make debug`
- `make release`
- `make test`
- `make fuzz`
- `make asan`
- `make install`

Artefakte:
- `librtmp2.so`
- `librtmp2.a`
- `librtmp2.dll`
- `librtmp2.lib`
- pkg-config file `librtmp2.pc`

***

## Releases und Versionierung

SemVer:
- `0.x` solange API/ABI noch bewegt wird
- `1.0.0` sobald Header/API stabil sind

Geplante erste Versionen:
- `0.1.0` Legacy RTMP Server minimal
- `0.2.0` Legacy RTMP Client minimal
- `0.3.0` E-RTMP v1 Receive
- `0.4.0` E-RTMP v1 Send
- `0.5.0` E-RTMP v2 Capability Layer
- `0.6.0` Multitrack / Reconnect / ModEx

***

## Phasenplan

### Phase 1 — Legacy Core MVP

- Handshake
- Chunk Reader/Writer
- Message Reassembly
- AMF0
- `connect` / `createStream` / `publish`
- Minimalserver-Beispiel

**Akzeptanzkriterium:** OBS kann H.264 zu `librtmp2` senden.

### Phase 2 — Client MVP

- Outbound Connect
- Publish-Flow
- Play-Flow rudimentär

**Akzeptanzkriterium:** `librtmp2`-Client kann zu SRS publizieren.

### Phase 3 — E-RTMP v1

- ExVideoTagHeader
- FourCC
- HDR / Metadata
- `fourCcList`

**Akzeptanzkriterium:** HEVC/AV1-Streams werden erkannt und korrekt geparsed. [4]

### Phase 4 — E-RTMP v2

- `capsEx`
- `videoFourCcInfoMap`
- Reconnect
- Multitrack
- ModEx

**Akzeptanzkriterium:** v2-Negotiation ohne Hard-Fails gegen bekannte Gegenstellen. [5]

### Phase 5 — Hardening

- ASan/UBSan
- Fuzzing
- ABI-Stabilität
- Packaging
- Docs

***

## Erfolgskriterien

`librtmp2` ist erfolgreich, wenn:

- es als eigenständige, kleine C-Library buildbar ist,
- OBS und FFmpeg dagegen getestet werden können,
- es Legacy RTMP und E-RTMP v1/v2 als wiederverwendbare Basis anbietet,
- andere Projekte darauf eigene Server oder Clients bauen können,
- und daraus später ein separates Produkt mit API und Stats-Seite entstehen kann.

***

## GitHub- und Organisationskonzept

Empfohlenes initiales Repository:

- `https://github.com/AlexanderWagnerDev/librtmp2`

Empfohlene Branches:
- `main`
- `develop`

Empfohlene Labels:
- `protocol`
- `legacy-rtmp`
- `e-rtmp-v1`
- `e-rtmp-v2`
- `amf`
- `chunking`
- `client`
- `server`
- `fuzzing`
- `interop`
- `good first issue`
