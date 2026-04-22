/**
 * Topic Echo — C++ reference capability for Ganglion.
 *
 * Demonstrates the wasi-sdk + wit-bindgen toolchain for C++ capability
 * authoring. This is the C++ version of gang-capability-topic-echo.
 *
 * Build:
 *   wit-bindgen c ./wit --world ganglion-capability --out-dir gen
 *   $WASI_SDK_PATH/bin/clang++ --target=wasm32-wasip1 -mexec-model=reactor \
 *       -fno-exceptions -I gen -o topic-echo.core.wasm \
 *       gen/ganglion_capability.c gen/ganglion_capability_component_type.o \
 *       src/main.cpp
 *   wasm-tools component embed ./wit topic-echo.core.wasm -o topic-echo.embedded.wasm
 *   wasm-tools component new topic-echo.embedded.wasm -o topic-echo.component.wasm
 */

/* When building with wit-bindgen, include the generated header:
 * #include "ganglion_capability.h"
 *
 * For this reference example, we define a standalone implementation
 * that demonstrates the algorithm without requiring the generated bindings.
 */

#include <cstdint>
#include <cstdio>
#include <cstring>

/**
 * Topic echo configuration.
 */
struct EchoConfig {
    const char** topics;
    int topic_count;
    int decimation;    /* Capture every Nth message (1 = all) */
    int max_messages;  /* Max per topic (0 = unlimited) */
};

/**
 * A single captured message.
 */
struct CapturedMessage {
    const char* topic;
    uint64_t sequence;
    const uint8_t* data;
    size_t data_len;
};

/**
 * Apply decimation to a message stream.
 *
 * Returns the number of messages captured into the output buffer.
 */
static int decimate_messages(
    const uint8_t** messages,
    const size_t* message_sizes,
    int message_count,
    int decimation,
    int max_messages,
    CapturedMessage* out,
    const char* topic
) {
    if (decimation < 1) decimation = 1;
    int captured = 0;
    uint64_t seq = 0;

    for (int i = 0; i < message_count; i++) {
        if (i % decimation != 0) continue;
        if (max_messages > 0 && captured >= max_messages) break;

        seq++;
        out[captured].topic = topic;
        out[captured].sequence = seq;
        out[captured].data = messages[i];
        out[captured].data_len = message_sizes[i];
        captured++;
    }

    return captured;
}

/**
 * Build a JSON response from captured messages.
 *
 * In a real wit-bindgen component, this would use the canonical ABI
 * to return result<list<u8>, string>. Here we build a JSON string
 * to demonstrate the output format.
 */
static size_t build_json_response(
    const CapturedMessage* messages,
    int count,
    int decimation,
    char* buf,
    size_t buf_size
) {
    /* Simple JSON construction without a library */
    size_t pos = 0;
    pos += (size_t)snprintf(buf + pos, buf_size - pos,
        "{\"decimation\":%d,\"captured\":%d,\"messages\":[",
        decimation, count);

    for (int i = 0; i < count && pos < buf_size - 64; i++) {
        if (i > 0) buf[pos++] = ',';
        pos += (size_t)snprintf(buf + pos, buf_size - pos,
            "{\"topic\":\"%s\",\"seq\":%llu,\"size\":%zu}",
            messages[i].topic,
            (unsigned long long)messages[i].sequence,
            messages[i].data_len);
    }

    pos += (size_t)snprintf(buf + pos, buf_size - pos, "]}");
    return pos;
}

/*
 * Entry point for the WASM component.
 *
 * With wit-bindgen, the actual signature would be:
 *
 *   bool exports_ganglion_capability_run(
 *       ganglion_capability_list_string_t *args,
 *       ganglion_capability_result_list_u8_string_t *ret
 *   );
 *
 * This standalone version demonstrates the algorithm. When building
 * with wit-bindgen, replace the body of the generated function with
 * calls to the host imports and this decimation logic.
 *
 * Example with host imports:
 *
 *   // Subscribe to topic via host
 *   ganglion_capability_list_u8_t data;
 *   ganglion_capability_string_t err;
 *   bool ok = ganglion_capability_ros_interface_topic_subscribe(
 *       &topic_str, &data, &err);
 *
 *   // Decimate
 *   decimate_messages(...);
 *
 *   // Return JSON result via canonical ABI
 *   ret->is_err = false;
 *   ret->val.ok.ptr = (uint8_t*)buf;
 *   ret->val.ok.len = len;
 */

/* Demonstrate the algorithm with sample data */
#ifdef STANDALONE_TEST
#include <cstdio>

int main() {
    /* Sample messages */
    const uint8_t msg1[] = {0x01, 0x02, 0x03};
    const uint8_t msg2[] = {0x04, 0x05};
    const uint8_t msg3[] = {0x06, 0x07, 0x08, 0x09};
    const uint8_t msg4[] = {0x0a};

    const uint8_t* messages[] = {msg1, msg2, msg3, msg4};
    size_t sizes[] = {3, 2, 4, 1};

    CapturedMessage captured[4];
    int count = decimate_messages(
        messages, sizes, 4,
        2,   /* decimation: every 2nd message */
        0,   /* max: unlimited */
        captured,
        "/odom"
    );

    char buf[1024];
    size_t len = build_json_response(captured, count, 2, buf, sizeof(buf));
    buf[len] = '\0';

    printf("%s\n", buf);
    printf("Captured %d of 4 messages with decimation=2\n", count);

    return 0;
}
#endif
