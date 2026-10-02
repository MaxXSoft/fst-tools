#ifndef FSTAPI_FST_TOOLS_H_
#define FSTAPI_FST_TOOLS_H_

#include "fstapi.h"

/* Project glue: upstream libfst's public ABI is left unchanged. */
int fstToolsReaderDumpToVcdFile(fstReaderContext *ctx, const char *path);

#endif /* FSTAPI_FST_TOOLS_H_ */
