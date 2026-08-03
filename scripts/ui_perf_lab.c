#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

enum { ROW_COUNT = 10000, COMMAND_COUNT = ROW_COUNT * 3 };

typedef struct {
    uint32_t name_id;
    uint16_t icon_id;
    uint16_t flags;
    float y;
} Row;

typedef struct {
    float rect[4];
    uint32_t color;
    uint32_t resource;
} DrawInst;

static volatile uint64_t sink;

static double elapsed_us(LARGE_INTEGER start, LARGE_INTEGER end, LARGE_INTEGER frequency,
                         int iterations) {
    return (double)(end.QuadPart - start.QuadPart) * 1000000.0 /
           (double)frequency.QuadPart / (double)iterations;
}

int main(void) {
    Row *rows = (Row *)malloc(sizeof(Row) * ROW_COUNT);
    DrawInst *commands = (DrawInst *)malloc(sizeof(DrawInst) * COMMAND_COUNT);
    if (!rows || !commands) return 2;

    for (uint32_t i = 0; i < ROW_COUNT; ++i) {
        rows[i] = (Row){i, (uint16_t)(i & 1), (uint16_t)(i & 7), (float)i * 24.0f};
    }

    LARGE_INTEGER frequency, start, end;
    QueryPerformanceFrequency(&frequency);

    const int scan_iterations = 5001;
    QueryPerformanceCounter(&start);
    uint64_t scan_checksum = 0;
    for (int iteration = 0; iteration < scan_iterations; ++iteration) {
        uint64_t sum = 0;
        for (uint32_t i = 0; i < ROW_COUNT; ++i) {
            rows[i].y += 0.25f;
            rows[i].flags ^= 1;
            sum += rows[i].name_id + rows[i].icon_id + rows[i].flags;
        }
        scan_checksum ^= sum;
    }
    QueryPerformanceCounter(&end);
    double scan_us = elapsed_us(start, end, frequency, scan_iterations);
    sink = scan_checksum;

    const int emit_iterations = 2001;
    QueryPerformanceCounter(&start);
    uint64_t emit_checksum = 0;
    for (int iteration = 0; iteration < emit_iterations; ++iteration) {
        for (uint32_t i = 0; i < ROW_COUNT; ++i) {
            float y = rows[i].y;
            uint32_t at = i * 3;
            commands[at] = (DrawInst){{0.0f, y, 900.0f, y + 24.0f}, 0x101010ffu, 0};
            commands[at + 1] =
                (DrawInst){{8.0f, y + 4.0f, 24.0f, y + 20.0f}, 0xffffffffu, rows[i].icon_id};
            commands[at + 2] =
                (DrawInst){{30.0f, y, 880.0f, y + 24.0f}, 0xffffffffu, rows[i].name_id};
        }
        emit_checksum ^= commands[COMMAND_COUNT - 1].resource ^
                         commands[COMMAND_COUNT - 1].color ^
                         (uint64_t)commands[COMMAND_COUNT - 1].rect[1] ^ COMMAND_COUNT;
        sink = commands[iteration % COMMAND_COUNT].resource;
    }
    QueryPerformanceCounter(&end);
    double emit_us = elapsed_us(start, end, frequency, emit_iterations);
    sink ^= emit_checksum;

    printf("10,000-row isolated C lab (same contiguous data model)\n");
    printf("contiguous scan/update      %9.2f us   checksum=%llu\n", scan_us,
           (unsigned long long)scan_checksum);
    printf("emit 30k flat draw records %9.2f us   checksum=%llu\n", emit_us,
           (unsigned long long)emit_checksum);
    printf("sizeof Row=%zu DrawInst=%zu sink=%llu\n", sizeof(Row), sizeof(DrawInst),
           (unsigned long long)sink);

    free(commands);
    free(rows);
    return 0;
}
