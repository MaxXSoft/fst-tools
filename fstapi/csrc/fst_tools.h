#ifndef FSTAPI_FST_TOOLS_H_
#define FSTAPI_FST_TOOLS_H_

#include "fstapi.h"

/* Project glue: upstream libfst's public ABI is left unchanged. */
int fstToolsReaderDumpToVcdFile(fstReaderContext *ctx, const char *path);

/* Cooperative cancellation, implemented by the build-local reader patch.
 * The flag may be changed by a callback on this thread, never concurrently. */
int fstToolsReaderIterBlocksControlled(
    fstReaderContext *ctx,
    void (*callback)(void *, uint64_t, fstHandle, const unsigned char *),
    void (*callback_varlen)(void *, uint64_t, fstHandle, const unsigned char *,
                            uint32_t),
    void *data, FILE *fv, const int *cancelled);

#endif /* FSTAPI_FST_TOOLS_H_ */
