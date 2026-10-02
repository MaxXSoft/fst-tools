#include "fst_tools.h"

/* The pinned libfst writes signal_typs[maxhandle] for a real alias without
 * growing the table. Preflight through the public iterator, which does not
 * perform that write, before ProcessHier or opening/truncating the output.
 * Remove this guard when the corresponding upstream fix is incorporated. */
static int fstToolsVcdHierarchyIsSafe(fstReaderContext *ctx) {
  if (!fstReaderIterateHierRewind(ctx)) return 0;

  uint64_t maxhandle = 0;
  uint64_t capacity = 65536;
  int safe = 1;
  struct fstHier *hier;
  while ((hier = fstReaderIterateHier(ctx))) {
    if (hier->htyp != FST_HT_VAR) continue;
    if (!hier->u.var.is_alias) {
      if (maxhandle == capacity) capacity *= 2;
      maxhandle++;
    } else if (maxhandle == capacity &&
               (hier->u.var.typ == FST_VT_VCD_REAL ||
                hier->u.var.typ == FST_VT_VCD_REAL_PARAMETER ||
                hier->u.var.typ == FST_VT_VCD_REALTIME ||
                hier->u.var.typ == FST_VT_SV_SHORTREAL)) {
      safe = 0;
      break;
    }
  }
  return fstReaderIterateHierRewind(ctx) && safe;
}

int fstToolsReaderDumpToVcdFile(fstReaderContext *ctx, const char *path) {
  if (!ctx || !fstToolsVcdHierarchyIsSafe(ctx)) return 1;

  FILE *file = path ? fopen(path, "wb") : stdout;
  if (!file) return 1;

  char *buffer = NULL;
  if (path) {
    buffer = malloc(2 * 1024 * 1024);
    if (buffer) setvbuf(file, buffer, _IOFBF, 2 * 1024 * 1024);
  }

  int success = fstReaderProcessHier(ctx, file);
  if (success) {
    /* ProcessHier rebuilds and clears the process mask. Export all the
     * signals whose declarations were just written to the VCD. */
    fstReaderSetFacProcessMaskAll(ctx);
    success = fstReaderIterBlocks(ctx, NULL, NULL, file);
  }
  if (ferror(file)) success = 0;
  if (path) {
    if (fclose(file)) success = 0;
    free(buffer);
  } else if (fflush(file)) {
    success = 0;
  }
  return success ? 0 : 1;
}
