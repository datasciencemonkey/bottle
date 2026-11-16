/* A C host for the R15 ABI (NFR-18a, NFR-18c). tests/c_host.rs compiles and runs it.
 * Exit 0 and print "ok" on success; any other exit code names the failed step. */
#include <stdio.h>
#include <string.h>

#include "mog_core.h"

static int contains(char *reply, const char *needle) {
    int found = reply != NULL && strstr(reply, needle) != NULL;
    mog_free_string(reply);
    return found;
}

static void on_event(const char *event_json, void *user_data) {
    (void)event_json;
    (void)user_data;
}

int main(void) {
    static const char request[] = "{\"command\":\"cli_status\",\"args\":{}}";
    static const char bad_utf8[] = {'{', (char)0xff, (char)0xfe, '}', 0};

    if (mog_abi_version() != MOG_ABI_VERSION) return 1;
    if (!contains(mog_call(request), "\"not_initialized\"")) return 2;
    if (mog_init(bad_utf8) != MOG_STATUS_INVALID_ARGUMENT) return 3;
    if (mog_init(NULL) != MOG_STATUS_OK) return 4;
    if (!contains(mog_call(request), "\"not_implemented\"")) return 5;
    if (!contains(mog_call(NULL), "\"invalid_argument\"")) return 6;
    if (!contains(mog_call(bad_utf8), "\"invalid_argument\"")) return 7;
    if (!contains(mog_call("not json"), "\"invalid_json\"")) return 8;
    if (mog_set_event_callback(on_event, NULL) != MOG_STATUS_OK) return 9;
    if (mog_set_event_callback(NULL, NULL) != MOG_STATUS_OK) return 10;
    mog_free_string(NULL);
    puts("ok");
    return 0;
}
