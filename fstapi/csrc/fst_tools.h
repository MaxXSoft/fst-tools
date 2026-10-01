#ifndef FST_TOOLS_H
#define FST_TOOLS_H

#include "fstapi.h"

/* Project glue: upstream libfst's public ABI is left unchanged. */
int fstToolsReaderDumpToVcdFile(fstReaderContext *ctx, const char *path);

#endif
