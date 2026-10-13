// C entry point for Recast's solo-mesh pipeline (as in RecastDemo's Sample_SoloMesh), used by src/navgen.rs.
#include "Recast.h"
#include <string.h>

struct Lp2Params {
    float cs, ch, slope;
    int walkableHeight, walkableClimb, walkableRadius, maxEdgeLen;
    float maxError;
    int minRegionArea, mergeRegionArea;
    float detailSampleDist, detailMaxError;
    float bmin[3], bmax[3];
};

// The detail mesh (rcPolyMeshDetail): nmeshes x (vertex base, vertex count, triangle base,
// triangle count), nverts x world xyz, ntris x (3 indices into the sub-mesh's vertices, flags).
struct Lp2Mesh {
    int nmeshes, nverts, ntris;
    const unsigned int* meshes;
    const float* verts;
    const unsigned char* tris;
    void* handle;
};

extern "C" void lp2_recast_free(Lp2Mesh* m) {
    rcFreePolyMeshDetail((rcPolyMeshDetail*)m->handle);
    m->handle = 0;
}

// Returns 0 on success, else the failing step (1-based).
extern "C" int lp2_recast_build(const float* verts, int nverts, const int* tris, int ntris, const unsigned char* walkable,
                                const Lp2Params* p, Lp2Mesh* out) {
    rcContext ctx(false);
    rcConfig cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.cs = p->cs;
    cfg.ch = p->ch;
    cfg.walkableSlopeAngle = p->slope;
    cfg.walkableHeight = p->walkableHeight;
    cfg.walkableClimb = p->walkableClimb;
    cfg.walkableRadius = p->walkableRadius;
    cfg.maxEdgeLen = p->maxEdgeLen;
    cfg.maxSimplificationError = p->maxError;
    cfg.minRegionArea = p->minRegionArea;
    cfg.mergeRegionArea = p->mergeRegionArea;
    cfg.maxVertsPerPoly = 6;
    rcVcopy(cfg.bmin, p->bmin);
    rcVcopy(cfg.bmax, p->bmax);
    rcCalcGridSize(cfg.bmin, cfg.bmax, cfg.cs, &cfg.width, &cfg.height);

    int step = 0;
    rcHeightfield* hf = rcAllocHeightfield();
    rcCompactHeightfield* chf = 0;
    rcContourSet* cset = 0;
    rcPolyMesh* pm = 0;
    rcPolyMeshDetail* dm = 0;
    unsigned char* areas = new unsigned char[ntris];
    memset(areas, 0, ntris);
    // Slope test, then drop triangles the caller marked as never walkable (they still block).
    rcMarkWalkableTriangles(&ctx, cfg.walkableSlopeAngle, verts, nverts, tris, ntris, areas);
    for (int i = 0; i < ntris; i++)
        if (!walkable[i]) areas[i] = RC_NULL_AREA;

    if (++step, !hf || !rcCreateHeightfield(&ctx, *hf, cfg.width, cfg.height, cfg.bmin, cfg.bmax, cfg.cs, cfg.ch)) goto done;
    if (++step, !rcRasterizeTriangles(&ctx, verts, nverts, tris, areas, ntris, *hf, cfg.walkableClimb)) goto done;
    rcFilterLowHangingWalkableObstacles(&ctx, cfg.walkableClimb, *hf);
    rcFilterLedgeSpans(&ctx, cfg.walkableHeight, cfg.walkableClimb, *hf);
    rcFilterWalkableLowHeightSpans(&ctx, cfg.walkableHeight, *hf);
    chf = rcAllocCompactHeightfield();
    if (++step, !chf || !rcBuildCompactHeightfield(&ctx, cfg.walkableHeight, cfg.walkableClimb, *hf, *chf)) goto done;
    if (++step, !rcErodeWalkableArea(&ctx, cfg.walkableRadius, *chf)) goto done;
    if (++step, !rcBuildDistanceField(&ctx, *chf)) goto done;
    if (++step, !rcBuildRegions(&ctx, *chf, 0, cfg.minRegionArea, cfg.mergeRegionArea)) goto done;
    cset = rcAllocContourSet();
    if (++step, !cset || !rcBuildContours(&ctx, *chf, cfg.maxSimplificationError, cfg.maxEdgeLen, *cset)) goto done;
    pm = rcAllocPolyMesh();
    if (++step, !pm || !rcBuildPolyMesh(&ctx, *cset, cfg.maxVertsPerPoly, *pm)) goto done;
    dm = rcAllocPolyMeshDetail();
    if (++step, !dm || !rcBuildPolyMeshDetail(&ctx, *pm, *chf, p->detailSampleDist, p->detailMaxError, *dm)) goto done;
    step = 0;
    out->nmeshes = dm->nmeshes;
    out->nverts = dm->nverts;
    out->ntris = dm->ntris;
    out->meshes = dm->meshes;
    out->verts = dm->verts;
    out->tris = dm->tris;
    out->handle = dm;
    dm = 0;
done:
    delete[] areas;
    rcFreeHeightField(hf);
    rcFreeCompactHeightfield(chf);
    rcFreeContourSet(cset);
    rcFreePolyMesh(pm);
    rcFreePolyMeshDetail(dm);
    return step;
}
