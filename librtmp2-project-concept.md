# librtmp2 — Projektkonzept

**Ziel:** Eine moderne, offene C/Go-Bibliothek für Legacy-RTMP und Enhanced RTMP v1/v2 (E-RTMP), server- und clientseitig, vergleichbar konsumierbar wie `libsrt`.

---

## 1. Vision und Marktlücke

Das Real-Time Messaging Protocol (RTMP) ist trotz seines Alters der dominierende Ingest-Standard bei Twitch, YouTube, Meta, Kick und nahezu allen anderen großen Streaming-Plattformen. Gleichzeitig gibt es keine moderne, saubere, isolierte Bibliothek die:

- Legacy RTMP (Adobe 2012) vollständig implementiert
- Enhanced RTMP v1 (Veovera 2023, finalisiert 2025) unterstützt
- Enhanced RTMP v2 (Veovera 2024, Beta) unterstützt
- Server- **und** Client-seitig verwendbar ist
- Wie `libsrt` als C-Library mit Bindings konsumierbar ist
- Eine offizielle Testsuite gegen OBS, FFmpeg und HaishinKit mitbringt

Alle existierenden Lösungen (SRS, Node-Media-Server, nginx-rtmp) sind vollständige Mediaserver, keine isolierten Bibliotheken. Wer RTMP in ein eigenes Produkt einbauen will, muss heute entweder einen kompletten Mediaserver als Dependency einschleppen oder das Protokoll von Grund auf selbst schreiben. Diese Lücke ist der Grund warum `librtmp2` existieren soll.

---

## 2. Sprachenwahl und Architekturentscheidung

### Primärsprache: Go

**Begründung:**
- Einfache Cross-Compilation für alle Zielplattformen (Linux, macOS, Windows, ARM)
- Exzellentes Netzwerk-IO-Modell (Goroutines, `net` package, keine Callback-Hölle)
- `cgo` ermöglicht das Exportieren als C-Library (`librtmp2.so` / `.dll`) für Downstream-Konsumenten in C, PHP, Python, Rust via FFI
- Einfacher als Rust für AI-Agenten Code-Generierung (explizitere Fehlerbehandlung, weniger Lifetime-Komplexität)
- Gut lesbar für externe Contributor

### Sekundäres Exportformat: C-ABI via `cgo`

Die öffentliche API wird als C-ABI exportiert sodass jede Sprache die FFI spricht die Bibliothek nutzen kann. Das ist exakt das Modell von `libsrt`.

---

## 3. Repository-Struktur

```
librtmp2/
│
├── README.md
├── LICENSE                        (MIT)
├── go.mod
├── go.sum
│
├── rtmp/                          # Kernprotokoll-Implementierung
│   ├── handshake.go               # C0/C1/C2/S0/S1/S2 Handshake
│   ├── chunk.go                   # Chunk Stream: Basic Header, Message Header Typ 0-3
│   ├── message.go                 # Message-Typen, Reassembly, Dispatching
│   ├── amf/                       # AMF0 + AMF3 Encoder/Decoder
│   │   ├── amf0.go
│   │   ├── amf3.go
│   │   └── amf_test.go
│   ├── command.go                 # Connect, CreateStream, Publish, Play, FCPublish, ...
│   ├── control.go                 # SetChunkSize, Ack, WindowAckSize, PeerBandwidth, UserControl
│   └── errors.go
│
├── ertmp/                         # Enhanced RTMP Erweiterungen
│   ├── v1/
│   │   ├── video.go               # ExVideoTagHeader, IsExHeader-Flag, PacketType-Logik
│   │   ├── fourcc.go              # FourCC-Werte: av01, vp09, hvc1, ...
│   │   ├── metadata.go            # PacketTypeMetadata, colorInfo, HDR (BT.2020)
│   │   └── audio.go              # Enhanced Audio: Opus, AC-3, E-AC-3 (v1)
│   └── v2/
│       ├── connect.go             # capsEx, videoFourCcInfoMap, Capability-Negotiation
│       ├── multitrack.go          # Multitrack-Handling (v2-spezifisch)
│       ├── reconnect.go           # Reconnect-Request-Mechanismus (Server→Client)
│       ├── modex.go               # ModEx-Erweiterungen
│       └── audio_v2.go           # Erweiterte Audio-Codec-Integration v2
│
├── server/                        # Server-seitige Logik
│   ├── server.go                  # TCP-Listener, Connection-Accept-Loop
│   ├── conn.go                    # Pro-Connection State Machine
│   ├── session.go                 # Stream-Lifecycle: Publish, Play, Stop
│   └── hooks.go                  # Callback-Hooks: OnConnect, OnPublish, OnFrame, OnClose
│
├── client/                        # Client-seitige Logik
│   ├── client.go                  # Verbindung zu RTMP-Server, Handshake
│   ├── publish.go                 # Stream publizieren (OBS-Äquivalent)
│   └── play.go                   # Stream konsumieren
│
├── cabi/                          # C-ABI Export via cgo
│   ├── librtmp2.go               # //export Direktiven für alle Public-Funktionen
│   ├── librtmp2.h                # Auto-generierter C-Header (via `go tool cgo`)
│   └── Makefile                   # Build-Target für .so / .dll / .a
│
├── examples/
│   ├── simple-server/             # Minimaler Server der einen Stream annimmt
│   │   └── main.go
│   ├── simple-client/             # Minimaler Client der einen Stream sendet
│   │   └── main.go
│   └── ertmp-v2-server/          # Server mit vollständiger E-RTMP-v2-Unterstützung
│       └── main.go
│
├── tests/
│   ├── unit/                      # Unit-Tests für alle Protokollschichten
│   ├── integration/               # Integrationstests gegen echte Gegenstellen
│   │   ├── obs_test.go            # OBS-Kompatibilitätstest (via lokales OBS)
│   │   ├── ffmpeg_test.go         # FFmpeg push/pull Tests
│   │   └── haishinkit_test.go    # HaishinKit iOS/Android Tests
│   └── fixtures/                  # Aufgezeichnete Packet-Captures (.pcap) als Testdaten
│
├── docs/
│   ├── protocol-notes.md          # Interne Protokoll-Notizen und Spec-Abweichungen
│   ├── ertmp-v1-mapping.md       # Mapping Veovera-Spec → Code-Stellen
│   ├── ertmp-v2-mapping.md       # Mapping Veovera-Spec v2 → Code-Stellen
│   └── api-reference.md          # Öffentliche API-Dokumentation
│
└── .github/
    └── workflows/
        ├── ci.yml                 # Tests auf Linux/macOS/Windows
        ├── interop.yml            # Interop-Tests gegen OBS, FFmpeg (nightly)
        └── release.yml            # Tag → Release + Build von .so/.dll/.a
```

---

## 4. Protokollschichten im Detail

### 4.1 Handshake (Legacy RTMP)

```
Client → Server: C0 (1 Byte, Version=3)
Client → Server: C1 (1536 Bytes, Timestamp + Random)
Server → Client: S0 (1 Byte, Version=3)
Server → Client: S1 (1536 Bytes)
Server → Client: S2 (1536 Bytes, Echo von C1)
Client → Server: C2 (1536 Bytes, Echo von S1)
```

**Implementierungsaufgaben:**
- `handshake.go`: `ServerHandshake(conn net.Conn) error` und `ClientHandshake(conn net.Conn) error`
- Komplexer Handshake (HMAC-SHA256, Adobe-Erweiterung) als optionale Variante
- Simple Handshake (keine Kryptographie) als Standard für maximale Kompatibilität

### 4.2 Chunk Stream

RTMP multiplext Nachrichten über Chunk Streams. Jeder Chunk hat:

- **Basic Header** (1–3 Bytes): `fmt` (2 Bit) + `csid` (Chunk Stream ID)
- **Message Header** (Typ 0: 11 Bytes, Typ 1: 7 Bytes, Typ 2: 3 Bytes, Typ 3: 0 Bytes)
- **Extended Timestamp** (4 Bytes, nur wenn Timestamp ≥ 0xFFFFFF)
- **Payload** (bis zu `chunk_size` Bytes pro Chunk)

**Implementierungsaufgaben:**
- `chunk.go`: `ReadChunk(r io.Reader, state *ChunkState) (*Chunk, error)`
- `chunk.go`: `WriteChunk(w io.Writer, msg *Message, chunkSize int) error`
- `ChunkState` hält pro csid den letzten Timestamp, MessageLength, MessageTypeID, MessageStreamID für Delta-Encoding (Typ 1/2/3)
- `SetChunkSize`-Nachrichten müssen sofort den internen Puffer anpassen

### 4.3 Message-Typen

| TypeID | Name | Handling |
|--------|------|----------|
| 1 | SetChunkSize | Sofort anwenden |
| 2 | Abort | Chunk-Buffer für csid leeren |
| 3 | Acknowledgement | ACK-Logik |
| 4 | UserControl | StreamBegin, StreamEOF, PingRequest/Response |
| 5 | WindowAckSize | Fenstergröße setzen |
| 6 | PeerBandwidth | Bandbreite signalisieren |
| 8 | Audio | An AudioHandler weiterleiten |
| 9 | Video | An VideoHandler weiterleiten (Legacy oder E-RTMP) |
| 15/18 | DataMessage AMF3/AMF0 | onMetaData, onTextData |
| 17/20 | CommandMessage AMF3/AMF0 | connect, createStream, publish, play, FCPublish |

### 4.4 AMF0 und AMF3

AMF0-Typen die vollständig implementiert werden müssen:
- Number (Float64), Boolean, String, Object, Null, Undefined, Reference, ECMAArray, StrictArray, Date, LongString, Unsupported, XMLDocument, TypedObject

AMF3 zusätzlich:
- Integer, ByteArray, VectorInt/UInt/Double/Object, Dictionary

**Implementierungsaufgaben:**
- `amf/amf0.go`: `Encode(v interface{}) ([]byte, error)` und `Decode(r io.Reader) (interface{}, error)`
- `amf/amf3.go`: Analog
- Beide Varianten müssen im Connect-Handshake verhandelt werden (objectEncoding Property)

---

## 5. Enhanced RTMP v1 — Erweiterungen

### 5.1 IsExHeader-Flag (Kern der Erweiterung)

Das erste Nibble (UB[4]) des VideoTagHeaders wird neu interpretiert:

```
IF (UB[4] & 0b1000) != 0:
    IsExHeader = true
    PacketType = UB[4] & 0b0111   # nicht mehr CodecID
    FourCC     = UI32              # nächste 4 Bytes = Codec-Identifier
ELSE:
    IsExHeader = false
    FrameType  = UB[4] & 0b0111
    CodecID    = nächstes Nibble   # Legacy-Pfad
```

**Implementierungsaufgaben (`ertmp/v1/video.go`):**
- `ParseVideoTagHeader(data []byte) (*VideoTagHeader, error)` — erkennt Legacy vs. Enhanced
- `PacketType`-Enum: `SequenceStart=0`, `CodedFrames=1`, `SequenceEnd=2`, `CodedFramesX=3`, `Metadata=4`, `MPEG2TSSequenceStart=5`
- FourCC-Tabelle: `av01` (AV1), `vp09` (VP9), `hvc1` (HEVC), erweiterbar

### 5.2 FourCC-basierte Codec-Unterstützung

| FourCC | Codec | SequenceStart-Payload |
|--------|-------|----------------------|
| `av01` | AV1 | AV1CodecConfigurationRecord |
| `vp09` | VP9 | VPCodecConfigurationRecord |
| `hvc1` | HEVC | HEVCDecoderConfigurationRecord (ISO 14496-15) |
| `avc1` | H.264 | AVCDecoderConfigurationRecord (Legacy) |

### 5.3 HDR-Metadaten (colorInfo)

`PacketTypeMetadata` transportiert ein AMF-Objekt:

```
colorInfo = {
  colorConfig: {
    bitDepth: Number,              // 8, 10 oder 12
    colorPrimaries: Number,        // ISO/IEC 23091-4, [0-255]
    transferCharacteristics: Number,
    matrixCoefficients: Number
  },
  hdrCll: {
    maxFall: Number,               // [0.0001-10000] cd/m²
    maxCLL: Number
  },
  hdrMdcv: {
    redX, redY, greenX, greenY, blueX, blueY,
    whitePointX, whitePointY: Number,
    maxLuminance: Number,          // [5-10000] nits
    minLuminance: Number           // [0.0001-5] nits
  }
}
```

### 5.4 Connect-Command-Erweiterung

Im `connect`-Command-Object wird ein neues Feld `fourCcList` ergänzt:

```
fourCcList: ["av01", "vp09", "hvc1"]   // StrictArray of Strings
```

Server antwortet mit den tatsächlich unterstützten FourCC-Werten.

---

## 6. Enhanced RTMP v2 — Erweiterungen

### 6.1 Capability-Negotiation (`capsEx`)

v2 erweitert die Connect-Phase um explizite Server-Capabilities:

```
videoFourCcInfoMap: {
  "av01": { isSupportedByServer: true },
  "hvc1": { isSupportedByServer: true },
  "vp09": { isSupportedByServer: false }
}
```

**Implementierungsaufgaben (`ertmp/v2/connect.go`):**
- `ParseCapsEx(obj AMFObject) (*Capabilities, error)`
- `BuildServerConnectResponse(caps *Capabilities) AMFObject`

### 6.2 Reconnect-Request-Mechanismus

Server kann dem Client mitteilen sich neu zu verbinden (z.B. bei Wartung oder Load-Balancing):

```
Server → Client: UserControl-Message Typ=ReconnectRequest
  Payload: { targetURL: "rtmp://newserver/live/streamkey", reason: "maintenance" }
```

**Implementierungsaufgaben (`ertmp/v2/reconnect.go`):**
- `SendReconnectRequest(conn *Conn, targetURL string, reason string) error`
- Client-seitig: `OnReconnectRequest`-Hook der von der Anwendung implementiert werden kann

### 6.3 Multitrack

v2 ermöglicht mehrere Video/Audio-Tracks pro Verbindung:

```
TrackID  uint8      // 0 = primärer Track
TrackDescriptor {
  codec   FourCC,
  role    string,   // "main", "alt", "thumbnail"
}
```

**Implementierungsaufgaben (`ertmp/v2/multitrack.go`):**
- `TrackManager` der per Verbindung aktive Tracks verwaltet
- `FrameRouter` der eingehende Frames anhand `TrackID` an die richtige Handler-Callback-Kette leitet

### 6.4 ModEx

ModEx erlaubt Protokollerweiterungen ohne Breaking Change durch ein generisches Extensibility-Frame:

**Implementierungsaufgaben (`ertmp/v2/modex.go`):**
- `ParseModEx(data []byte) (*ModExFrame, error)`
- Unbekannte ModEx-Typen MÜSSEN graceful ignoriert werden (kein Hard-Fail)

---

## 7. Öffentliche API (Go)

### 7.1 Server-Nutzung

```go
import "github.com/librtmp2/librtmp2/server"

srv := server.New(server.Config{
    Addr:          ":1935",
    ChunkSize:     4096,
    ERTMPv1:       true,
    ERTMPv2:       true,
})

srv.OnPublish(func(ctx *server.PublishContext) error {
    log.Printf("Stream gestartet: app=%s key=%s codec=%s",
        ctx.App, ctx.StreamKey, ctx.VideoCodec)
    return nil  // nil = erlaubt, error = ablehnen
})

srv.OnFrame(func(ctx *server.FrameContext) error {
    // ctx.Frame.Type: Audio | Video
    // ctx.Frame.IsKeyframe
    // ctx.Frame.FourCC (bei E-RTMP)
    // ctx.Frame.Data []byte
    return nil
})

srv.OnClose(func(ctx *server.CloseContext) {
    log.Printf("Stream beendet: %s nach %s", ctx.StreamKey, ctx.Duration)
})

log.Fatal(srv.ListenAndServe())
```

### 7.2 Client-Nutzung (Publish)

```go
import "github.com/librtmp2/librtmp2/client"

c, err := client.Dial("rtmp://localhost/live/mykey", client.Config{
    ERTMPv1: true,
    ERTMPv2: true,
    FourCCList: []string{"av01", "hvc1"},
})

pub, err := c.Publish("live", "mykey")

// Frames senden
pub.WriteVideoFrame(&client.VideoFrame{
    FourCC:    "av01",
    IsKeyframe: true,
    Data:      avifData,
    Timestamp: time.Now(),
})

pub.Close()
```

### 7.3 C-ABI (via cgo)

```c
#include "librtmp2.h"

// Server erstellen
LRTMP2_Server* srv = lrtmp2_server_new(":1935");
lrtmp2_server_set_ertmpv2(srv, 1);
lrtmp2_server_on_publish(srv, my_on_publish_callback, userdata);
lrtmp2_server_on_frame(srv, my_on_frame_callback, userdata);
lrtmp2_server_listen(srv);  // blockierend
```

---

## 8. State Machine: Server-Connection

Jede eingehende TCP-Verbindung durchläuft folgende Zustände:

```
[Init]
  │ TCP Accept
  ▼
[Handshake]
  │ C0/C1/C2 ↔ S0/S1/S2 erfolgreich
  ▼
[Connected]
  │ connect-Command empfangen + validiert
  ▼
[StreamCreated]
  │ createStream-Command
  ▼
[Publishing]   ← publish-Command (OBS, FFmpeg, ...)
  │ oder
[Playing]      ← play-Command (Consumer)
  │
  │ Frames fließen
  │
[Closing]      ← FCUnpublish / deleteStream / TCP-Close
  ▼
[Closed]
```

Für E-RTMP v2 kommt ein zusätzlicher Zustand:

```
[Connected]
  │ capsEx-Negotiation
  ▼
[Negotiated]   ← Capabilities beider Seiten bekannt
  │
  ▼ weiter wie oben
```

---

## 9. Testsuite

### 9.1 Unit-Tests

Jede Protokollschicht hat eigene Unit-Tests mit Byte-level-Fixtures:

- `handshake_test.go`: Korrekter C0/C1/C2-Austausch mit aufgezeichneten Bytes
- `chunk_test.go`: Chunk-Reassembly, alle 4 Header-Typen, Extended Timestamp
- `amf_test.go`: Encode/Decode-Roundtrips für alle AMF0/AMF3-Typen
- `video_test.go`: Legacy VideoTagHeader und ExVideoTagHeader, alle PacketTypes
- `fourcc_test.go`: FourCC-Erkennung, SequenceStart-Payload-Parsing

### 9.2 Integrationstests

Aufgezeichnete `.pcap`-Dateien von echten Gegenstellen als Golden-Reference:

- OBS Studio → librtmp2-Server: H.264, HEVC, AV1
- FFmpeg `rtmp://` → librtmp2-Server: verschiedene Codecs
- librtmp2-Client → SRS-Server: Publish-Kompatibilität
- HaishinKit (iOS) → librtmp2-Server: Mobile-Client-Kompatibilität

### 9.3 Fuzzing

```go
// tests/fuzz/fuzz_chunk.go
func FuzzChunkParse(f *testing.F) {
    f.Fuzz(func(t *testing.T, data []byte) {
        // Muss bei beliebigen Bytes ohne Panic terminieren
        ParseChunk(bytes.NewReader(data), &ChunkState{})
    })
}
```

Fuzz-Targets für: Handshake-Parser, Chunk-Parser, AMF-Decoder, VideoTagHeader-Parser.

---

## 10. Fehlerbehandlung

Alle Fehler sind typisiert:

```go
type RTMPError struct {
    Code    ErrorCode
    Message string
    Cause   error
}

const (
    ErrHandshakeFailed    ErrorCode = iota
    ErrInvalidChunkHeader
    ErrAMFDecodeFailed
    ErrStreamKeyRejected
    ErrProtocolViolation
    ErrConnectionReset
    ErrUnsupportedFourCC
    ErrModExUnknown       // kein Hard-Fail, nur Logging
)
```

Unbekannte E-RTMP-Erweiterungen (ModEx, unbekannte PacketTypes) DÜRFEN NIE einen Hard-Error produzieren — graceful degradation ist Pflicht.

---

## 11. Konfiguration

```go
type Config struct {
    // Netzwerk
    Addr        string        // default: ":1935"
    ReadTimeout  time.Duration // default: 30s
    WriteTimeout time.Duration // default: 30s

    // Protokoll
    ChunkSize   int  // default: 4096, max: 65536
    WindowSize  int  // default: 2500000 Bytes
    ERTMPv1     bool // Enhanced RTMP v1 aktivieren
    ERTMPv2     bool // Enhanced RTMP v2 aktivieren (setzt v1 voraus)

    // Codecs (Server: was akzeptiert wird; Client: was gesendet wird)
    SupportedFourCCs []string // default: ["avc1", "av01", "hvc1", "vp09"]
    SupportedAudio   []string // default: ["mp4a", "Opus", "ac-3", "ec-3"]

    // Callbacks / Hooks
    AuthFunc    func(app, streamKey string) error
    MaxStreams   int // 0 = unbegrenzt
}
```

---

## 12. Build und Distribution

### Go-Modul (direkter Go-Einsatz)

```bash
go get github.com/librtmp2/librtmp2
```

### Shared Library für C/PHP/Python

```bash
# Baut librtmp2.so (Linux) / librtmp2.dylib (macOS) / librtmp2.dll (Windows)
make lib
```

```makefile
# cabi/Makefile
lib:
    go build -buildmode=c-shared -o librtmp2.so ./cabi/
    go build -buildmode=c-archive -o librtmp2.a ./cabi/
```

### Docker (für Tests)

```dockerfile
FROM golang:1.24-alpine AS builder
WORKDIR /app
COPY . .
RUN go build -o /librtmp2-server ./examples/simple-server

FROM alpine:3.22
COPY --from=builder /librtmp2-server /usr/local/bin/
EXPOSE 1935
CMD ["librtmp2-server"]
```

---

## 13. GitHub Actions CI

### ci.yml

```yaml
on: [push, pull_request]
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        go: ["1.24"]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with: { go-version: ${{ matrix.go }} }
      - run: go test ./... -race -coverprofile=coverage.out
      - run: go vet ./...
      - run: go build -buildmode=c-shared -o librtmp2.so ./cabi/
```

### interop.yml (nightly)

```yaml
on:
  schedule: [{ cron: "0 2 * * *" }]
jobs:
  interop-ffmpeg:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: sudo apt-get install -y ffmpeg
      - run: go run ./examples/simple-server &
      - run: sleep 2
      # FFmpeg sendet einen Teststream für 10 Sekunden
      - run: ffmpeg -re -f lavfi -i testsrc=size=1280x720:rate=30 
                    -c:v libx264 -t 10 
                    -f flv rtmp://localhost/live/test
      - run: go test ./tests/integration/... -run TestFFmpeg
```

---

## 14. Phasenplan für AI-Agenten

Die Implementierung ist in unabhängige Phasen aufgeteilt, die sequenziell oder parallel von AI-Agenten bearbeitet werden können.

### Phase 1 — Fundament (Woche 1–2)
**Ziel:** Minimaler Server der OBS-Verbindungen annimmt und H.264 empfängt.

- [ ] Repository-Struktur anlegen
- [ ] `rtmp/handshake.go` — Simple Handshake (keine HMAC)
- [ ] `rtmp/chunk.go` — Chunk-Reader/Writer, alle 4 Header-Typen
- [ ] `rtmp/amf/amf0.go` — AMF0 Encoder/Decoder
- [ ] `rtmp/message.go` — Message-Reassembly aus Chunks
- [ ] `rtmp/command.go` — connect, createStream, publish, FCPublish
- [ ] `rtmp/control.go` — SetChunkSize, Ack, WindowAckSize, StreamBegin
- [ ] `server/server.go` + `server/conn.go` — TCP-Accept-Loop, State Machine
- [ ] `server/hooks.go` — OnPublish, OnFrame, OnClose Callbacks
- [ ] `examples/simple-server/` — Minimales Beispiel
- [ ] Unit-Tests für alle obigen Pakete
- [ ] `tests/fixtures/` — .pcap-Dateien von OBS-Session aufzeichnen

**Akzeptanzkriterium:** OBS kann via `rtmp://localhost/live/test` publizieren, Server empfängt H.264-Frames und loggt sie.

### Phase 2 — Client-Seite (Woche 3)
**Ziel:** librtmp2 kann selbst als Publisher auftreten.

- [ ] `client/client.go` — Verbindungsaufbau, Handshake
- [ ] `client/publish.go` — Publish-Sequenz (connect → createStream → publish)
- [ ] `rtmp/amf/amf3.go` — AMF3 Encoder/Decoder
- [ ] `examples/simple-client/` — FFmpeg-Äquivalent in Go
- [ ] Integration-Test: librtmp2-Client → SRS-Server

**Akzeptanzkriterium:** `go run ./examples/simple-client` sendet H.264 an SRS ohne Fehler.

### Phase 3 — Enhanced RTMP v1 (Woche 4–5)
**Ziel:** HEVC, AV1, VP9 via E-RTMP v1 empfangen.

- [ ] `ertmp/v1/video.go` — ExVideoTagHeader-Parser, IsExHeader-Logik
- [ ] `ertmp/v1/fourcc.go` — FourCC-Registry
- [ ] `ertmp/v1/metadata.go` — PacketTypeMetadata, colorInfo/HDR
- [ ] `ertmp/v1/audio.go` — Opus, AC-3, E-AC-3 Audio-Handling
- [ ] Connect-Command um `fourCcList` erweitern
- [ ] Unit-Tests mit aufgezeichneten E-RTMP-v1-Streams
- [ ] Interop-Test: OBS (HEVC) → librtmp2

**Akzeptanzkriterium:** OBS mit HEVC-Codec kann verbinden, Server erkennt `hvc1` FourCC korrekt.

### Phase 4 — Enhanced RTMP v2 (Woche 6–8)
**Ziel:** Vollständige v2-Capability-Negotiation, Reconnect, Multitrack.

- [ ] `ertmp/v2/connect.go` — capsEx, videoFourCcInfoMap
- [ ] `ertmp/v2/reconnect.go` — ReconnectRequest Server→Client
- [ ] `ertmp/v2/multitrack.go` — TrackID, TrackManager, FrameRouter
- [ ] `ertmp/v2/modex.go` — ModEx-Parser, graceful ignore unbekannter Typen
- [ ] `examples/ertmp-v2-server/` — Vollständiges v2-Beispiel
- [ ] Interop-Tests gegen alle bekannten v2-Implementierungen

**Akzeptanzkriterium:** Server und Client beherrschen die vollständige v2-Handshake-Sequenz inkl. Capability-Negotiation.

### Phase 5 — C-ABI und Distribution (Woche 9–10)
**Ziel:** Bibliothek ist für Nicht-Go-Projekte konsumierbar.

- [ ] `cabi/librtmp2.go` — `//export`-Wrapper für alle Public-Funktionen
- [ ] `cabi/librtmp2.h` — C-Header
- [ ] `cabi/Makefile` — .so / .dll / .a Targets
- [ ] PHP-Binding-Beispiel via FFI
- [ ] Python-Binding-Beispiel via ctypes
- [ ] Dokumentation: `docs/api-reference.md`

### Phase 6 — Hardening und Release (Woche 11–12)
**Ziel:** Produktionsreife, Community-ready.

- [ ] Fuzz-Tests für alle Parser
- [ ] Backpressure-Handling bei langsamen Clients
- [ ] Speicherleck-Analyse (pprof)
- [ ] CHANGELOG.md, CONTRIBUTING.md
- [ ] GitHub Releases mit Pre-built Binaries und .so/.dll
- [ ] Docker Hub: `librtmp2/server:latest`

---

## 15. Verzeichnis der Spec-Referenzen

| Thema | Quelle | Abschnitt |
|-------|--------|-----------|
| RTMP Handshake | Adobe RTMP Spec 1.0 | Section 5.2 |
| Chunk Stream | Adobe RTMP Spec 1.0 | Section 6 |
| AMF0 | Adobe AMF0 Spec | vollständig |
| AMF3 | Adobe AMF3 Spec | vollständig |
| Message-Typen | Adobe RTMP Spec 1.0 | Section 7 |
| IsExHeader / ExVideoTagHeader | E-RTMP v1, Table 4 | ExVideoTagHeader |
| FourCC-Werte | E-RTMP v1, Table 4 | Video FourCC |
| colorInfo / HDR | E-RTMP v1 | Metadata Frame |
| fourCcList (Connect) | E-RTMP v1, Table 5 | Connect Object |
| videoFunction-Flags | E-RTMP v1, Table 6 | videoFunction |
| capsEx / videoFourCcInfoMap | E-RTMP v2 | Capability Negotiation |
| ReconnectRequest | E-RTMP v2 | Reconnect Mechanism |
| Multitrack | E-RTMP v2 | Multitrack |
| ModEx | E-RTMP v2 | ModEx |

---

## 16. Naming und Branding

- **Bibliotheksname:** `librtmp2` (libr + rtmp + 2)
- **Go-Modulpfad:** `github.com/AlexanderWagnerDev/librtmp2`
- **C-Header-Prefix:** `lrtmp2_`
- **Docker-Image:** `librtmp2/server`
- **Lizenz:** MIT
