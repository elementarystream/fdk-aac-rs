// Shims for the differential tests of fdk_aac_rs_fdk::huff_nodes.
// Every `cref_*` function defined at the start of a line is bound in
// fdk_aac_rs_cref::fdk_huff_nodes (see build.rs).
//
// The node tables are returned as SHORT arrays (all node structs consist of SHORTs). The
// file-static tables are taken from a copy of huff_nodes.cpp; the pointer tables are the ones of
// the reference library.

#include "cref_common.h"
#include "huff_nodes.h"

/* A copy of huff_nodes.cpp, for its file-static tables. */
namespace ref_hn {
#include "libFDK/src/huff_nodes.cpp"
}

#define HN_TABLE(sym)                                \
  if (strcmp(name, #sym) == 0) {                     \
    *n = (INT)(sizeof(ref_hn::sym) / sizeof(SHORT)); \
    return (const SHORT *)&ref_hn::sym;              \
  }

extern "C" {
const SHORT *cref_huff_nodes_table(const char *name, INT *n) {
  HN_TABLE(FDK_huffPart0Nodes)
  HN_TABLE(FDK_huffLavIdxNodes)
  HN_TABLE(FDK_huffICCNodes_h1D_0)
  HN_TABLE(FDK_huffICCNodes_h2D_0_0)
  HN_TABLE(FDK_huffICCNodes_h2D_0_1)
  HN_TABLE(FDK_huffICCNodes_h2D_1_0)
  HN_TABLE(FDK_huffICCNodes_h2D_1_1)
  HN_TABLE(FDK_huffCLDNodes_h1D_0)
  HN_TABLE(FDK_huffCLDNodes_h1D_1)
  HN_TABLE(FDK_huffCLDNodes_h2_0_0)
  HN_TABLE(FDK_huffCLDNodes_h2_0_1)
  HN_TABLE(FDK_huffCLDNodes_h2_1_0)
  HN_TABLE(FDK_huffCLDNodes_h2_1_1)
  HN_TABLE(FDK_huffReshapeNodes)
  HN_TABLE(FDK_huffIPDNodes)
  HN_TABLE(huffOLDNodes_h1D_0)
  HN_TABLE(huffOLDNodes_h1D_1)
  HN_TABLE(huffOLDNodes_h2D_0_0)
  HN_TABLE(huffOLDNodes_h2D_0_1)
  HN_TABLE(huffOLDNodes_h2D_1_0)
  HN_TABLE(huffOLDNodes_h2D_1_1)
  *n = 0;
  return NULL;
}
/* The node tables that the pointer tables of the reference library reference
   (kind: 0 ICC, 1 CLD, 2 OLD; dim: 1 or 2). */
const SHORT *cref_huff_nodes_pointer(INT kind, INT dim, INT i, INT j) {
  switch (kind) {
    case 0:
      return dim == 1 ? &FDK_huffICCNodes.h1D[i]->nodeTab[0][0]
                      : &FDK_huffICCNodes.h2D[i][j]->lav1[0][0];
    case 1:
      return dim == 1 ? &FDK_huffCLDNodes.h1D[i]->nodeTab[0][0]
                      : &FDK_huffCLDNodes.h2D[i][j]->lav3[0][0];
    default:
      return dim == 1 ? &huffOLDNodes.h1D[i]->nodeTab[0][0]
                      : &huffOLDNodes.h2D[i][j]->lav3[0][0];
  }
}
/* The global tables of the reference library. */
const SHORT *cref_huff_nodes_global(INT id) {
  switch (id) {
    case 0:
      return &FDK_huffPart0Nodes.cld[0][0];
    case 1:
      return &FDK_huffLavIdxNodes.nodeTab[0][0];
    case 2:
      return &FDK_huffReshapeNodes.nodeTab[0][0];
    default:
      return &FDK_huffIPDNodes.h1D[0].nodeTab[0][0];
  }
}
}
