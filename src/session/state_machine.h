#ifndef LRTMP2_SESSION_STATE_MACHINE_H
#define LRTMP2_SESSION_STATE_MACHINE_H

#include "librtmp2/types.h"

int lrtmp2_conn_transition(lrtmp2_conn_t *conn, lrtmp2_conn_state_t new_state);
const char *lrtmp2_conn_state_str(lrtmp2_conn_state_t state);

#endif
