# librtmp2 Server — Produktkonzept

## Zweck

`librtmp2-server` ist das spätere Produkt **oberhalb** der `librtmp2`-Core-Bibliothek. Es ist ein eigenständiger RTMP / E-RTMP-Server mit **API**, **Stats-Seite** und einer schlanken operativen Oberfläche. Die Produktlogik ist strikt getrennt von der Protokollimplementierung, damit der Core auch von anderen Projekten wiederverwendet werden kann. [1]

Das Ziel ist **kein** überladener Multi-Protokoll-Monolith wie MediaMTX, sondern ein fokussierter, moderner RTMP-/E-RTMP-Server mit sauberer UX, API-first-Denke und guter Observability. [1]

***

## Produktvision

Der Server soll das werden, was du ursprünglich beschrieben hast:

- Streams dynamisch per API anlegen
- Eine ordentliche Stats-Seite haben
- Fokussiert auf RTMP / E-RTMP bleiben
- Keine Drittplattform-Push-Targets benötigen
- Schlank, containerfreundlich und selbst hostbar sein

Die Bibliothek macht die Protokollarbeit, der Server macht das Produkt.

***

## Technische Schichtung

```text
OBS / FFmpeg / App
        │
        ▼
  librtmp2-server
  ├── RTMP Listener
  ├── Stream Registry
  ├── HTTP API
  ├── Stats Collector
  ├── Web UI
  └── Persistence
        │
        ▼
      librtmp2
      ├── Handshake
      ├── Chunking
      ├── AMF
      ├── RTMP Commands
      ├── E-RTMP v1
      └── E-RTMP v2
```

***

## Sprach- und Komponentenwahl

### Core
- `librtmp2` in **C**

### Server-Schicht
Empfohlene Optionen:

| Komponente | Empfehlung | Grund |
|---|---|---|
| API/HTTP | Go oder Python | gute Produktivität, saubere Web-Stacks |
| Web-UI | plain HTML/JS oder kleines Frontend | geringes Gewicht |
| Persistence | SQLite für Start, optional PostgreSQL | einfache erste Deployments |
| Containerisierung | Docker | passt zu deinem Workflow  |

Die Sprache der Server-Schicht kann später entschieden werden, weil sie **nicht** mehr die Protokollbasis definiert.

***

## Feature-Set v1

### 1. RTMP/E-RTMP Ingest

- Eingehende Publisher-Verbindungen annehmen
- Stream-Key validieren
- App-Name und Stream-Key extrahieren
- Codec/FourCC erkennen
- Session-Status speichern

### 2. HTTP API

Beispielendpunkte:

```text
POST   /api/v1/streams
GET    /api/v1/streams
GET    /api/v1/streams/:id
PATCH  /api/v1/streams/:id
DELETE /api/v1/streams/:id

GET    /api/v1/sessions
GET    /api/v1/sessions/:id
POST   /api/v1/sessions/:id/disconnect
POST   /api/v1/sessions/:id/reconnect

GET    /api/v1/stats/overview
GET    /api/v1/stats/streams/:id
GET    /api/v1/health
```

### 3. Stats-Seite

Die UI soll modern, klar und operativ nützlich sein:

- Aktive Streams
- Status: online/offline
- Uptime
- Eingehende Bitrate
- Frame Rate
- Video Codec / FourCC
- Audio Codec
- Anzahl Sessions
- Fehlerrate / letzte Fehler
- Optionale History-Charts

### 4. Stream Registry

Jeder Stream ist ein Objekt:

```json
{
  "id": "stream_123",
  "name": "Main Stage",
  "app": "live",
  "stream_key": "abc123",
  "enabled": true,
  "require_auth": true,
  "allowed_codecs": ["avc1", "hvc1", "av01"],
  "created_at": "2026-06-26T00:00:00Z"
}
```

### 5. Authentifizierung

Mindestens:
- statische API-Tokens
- Stream-Key-basierte Publish-Auth
- optional Basic Auth für Stats-Seite

***

## Architekturmodule

### Ingest Worker

Verarbeitet eingehende RTMP-Verbindungen und nutzt `librtmp2` intern. Übersetzt Library-Callbacks in Anwendungsevents.

### Session Manager

Hält aktive Verbindungen, Stream-Lifecycle, Online-Status, Disconnect-Reason.

### Stats Collector

Aggregiert Metriken pro Stream und Session:
- Bytes in/out
- Bitrate
- FPS grob geschätzt
- Codec
- Dauer
- Fehlerzähler

### REST API

Operative Steuerung und Datenabfrage.

### Web UI

Einfaches Frontend für Übersicht und Detailseiten.

### Persistence Layer

Mindestens Tabellen / Collections für:
- streams
- api_tokens
- session_history
- optional stats_samples

***

## Datenmodell

### streams

| Feld | Typ | Zweck |
|---|---|---|
| id | string | Primärschlüssel |
| name | string | Anzeigename |
| app | string | RTMP-App |
| stream_key | string | Publish-Key |
| enabled | bool | Aktiv/Inaktiv |
| require_auth | bool | Auth nötig |
| allowed_codecs | json/text | Erlaubte Codecs |
| created_at | timestamp | Erstellung |
| updated_at | timestamp | Änderung |

### sessions

| Feld | Typ | Zweck |
|---|---|---|
| id | string | Session-ID |
| stream_id | string | Bezug zu Stream |
| remote_addr | string | IP/Port |
| started_at | timestamp | Verbindungsbeginn |
| ended_at | timestamp | Verbindungsende |
| status | string | active/closed/error |
| video_codec | string | Codec/FourCC |
| audio_codec | string | Audioformat |
| bytes_in | bigint | empfangene Bytes |
| last_error | text | letzter Fehler |

### stats_samples

| Feld | Typ | Zweck |
|---|---|---|
| id | integer | PK |
| session_id | string | Bezug |
| ts | timestamp | Zeitpunkt |
| bitrate_in | integer | aktuelle Bitrate |
| fps | float | aktuelle FPS |
| keyframe_interval | float | geschätzt |

***

## API-Design-Prinzipien

- JSON only
- stabile Versionierung: `/api/v1`
- maschinenlesbare Fehlerobjekte
- klare Statuscodes
- idempotente GET/DELETE
- Auditierbarkeit für administrative Aktionen

Fehlerbeispiel:

```json
{
  "error": {
    "code": "STREAM_NOT_FOUND",
    "message": "The requested stream does not exist."
  }
}
```

***

## Stats-UI-Prinzipien

Die UI soll **nicht** wie SRS aussehen. Sie soll modern, reduziert und nützlich sein.

Ansichten:
- Dashboard
- Streams-Liste
- Stream-Detailseite
- Session-Detailseite
- System Health

Wichtige Elemente:
- Such- und Filterfunktion
- Farbcodierter Status
- Live aktualisierte Werte
- Verlauf über letzte Minuten/Stunden
- Mobile brauchbar, Desktop stark

***

## Deployment-Konzept

Empfohlene Struktur:

```text
services:
  librtmp2-server:
    image: alexanderwagnerdev/librtmp2-server:latest
    ports:
      - "1935:1935"
      - "8080:8080"
    volumes:
      - ./data:/data
      - ./config:/config
```

Ports:
- `1935/tcp` RTMP
- `8080/tcp` HTTP API + UI

***

## Roadmap

### Phase A — Tech Demo

- `librtmp2` minimal integrieren
- 1 Stream-Key fest verdrahtet
- 1 API-Route `GET /health`
- 1 Web-Page mit aktiven Sessions

### Phase B — MVP

- Stream Registry
- CRUD API
- Login / API Token
- Dashboard
- Session History

### Phase C — Produktreife

- Persistente Metrics
- Reconnect-Steuerung
- feinere Codec-Richtlinien
- bessere Charts
- Multi-Node-Fähigkeit

### Phase D — Cluster / Enterprise optional

- mehrere Ingest-Nodes
- zentrale Control Plane
- Node-Health
- Scheduling / Drain Mode

***

## Abgrenzung zu anderen Tools

### Gegenüber SRS

- schöneres Produkt
- API-first
- bessere UX
- klarer Fokus auf RTMP/E-RTMP
- eigener Core statt nur Wrapper langfristig

### Gegenüber MediaMTX

- weniger Protokollballast
- gezielter Use-Case
- modernere Stats- und API-Oberfläche
- kein generischer Alles-Router

### Gegenüber nginx-rtmp

- moderne Codecs / E-RTMP
- aktive Produktarchitektur
- API und Observability

***

## Erfolgskriterien

`librtmp2-server` ist erfolgreich, wenn:

- ein Nutzer in wenigen Minuten einen RTMP/E-RTMP-Server starten kann,
- Streams per API angelegt werden können,
- die Stats-Seite klarer und nützlicher ist als SRS,
- das Produkt klein und fokussiert bleibt,
- und der Server vollständig auf `librtmp2` basiert, nicht auf SRS.

***

## GitHub-Strategie

Empfohlene Repositories unter `AlexanderWagnerDev`:

- `AlexanderWagnerDev/librtmp2`
- `AlexanderWagnerDev/librtmp2-server`

Optional später:
- `AlexanderWagnerDev/librtmp2-python`
- `AlexanderWagnerDev/librtmp2-go`
- `AlexanderWagnerDev/librtmp2-obs-plugin`
