#include "fst_tools.h"

int fstToolsReaderDumpToVcdFile(fstReaderContext *ctx, const char *path)
{
    if (!ctx)
        return 1;

    FILE *file = path ? fopen(path, "wb") : stdout;
    if (!file)
        return 1;

    char *buffer = NULL;
    if (path) {
        buffer = malloc(2 * 1024 * 1024);
        if (buffer)
            setvbuf(file, buffer, _IOFBF, 2 * 1024 * 1024);
    }

    int success = fstReaderProcessHier(ctx, file);
    if (success) {
        /* ProcessHier rebuilds and clears the process mask. Export all the
         * signals whose declarations were just written to the VCD. */
        fstReaderSetFacProcessMaskAll(ctx);
        success = fstReaderIterBlocks(ctx, NULL, NULL, file);
    }
    if (ferror(file))
        success = 0;
    if (path) {
        if (fclose(file))
            success = 0;
        free(buffer);
    } else if (fflush(file)) {
        success = 0;
    }
    return success ? 0 : 1;
}
