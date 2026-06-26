/**
 * state_machine.c — Connection state machine helpers
 */
#include "session/state_machine.h"
#include "session/conn.h"
#include "core/log.h"
#include <string.h>
#include "librtmp2/types.h"

static const char *state_names[] = {
    "TCP_ACCEPTED", "HANDSHAKE", "CONNECTED", "CAPS_NEGOTIATED",
    "APP_CONNECTED", "STREAM_CREATED", "PUBLISHING", "PLAYING",
    "CLOSING", "CLOSED"
};

int lrtmp2_conn_transition(lrtmp2_conn_t *conn, lrtmp2_conn_state_t new_state)
{
    if (!conn) return LRTMP2_ERR_INTERNAL;

    if (new_state < conn->state) {
        LRTMP2_LOG_WARN("Backward state transition: %s -> %s",
                         state_names[conn->state], state_names[new_state]);
        return LRTMP2_ERR_PROTOCOL;
    }

    if (new_state >= LRTMP2_STATE_CLOSING) {
        LRTMP2_LOG_INFO("Connection closing: %s", state_names[new_state]);
    } else {
        LRTMP2_LOG_DEBUG("State transition: %s -> %s",
                          state_names[conn->state], state_names[new_state]);
    }

    conn->state = new_state;
    return LRTMP2_OK;
}

const char *lrtmp2_conn_state_str(lrtmp2_conn_state_t state)
{
    if (state <= LRTMP2_STATE_CLOSED) return state_names[state];
    return "UNKNOWN";
}
