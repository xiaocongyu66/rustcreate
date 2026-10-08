// Mesh buffer pool: size-classed free lists with canary guards (debug).
//
// Memory recovery contract with Rust:
// - mcv_meshbuf_acquire: reuse a cached block when possible (LIFO per class,
//   Treiber stack, thread-safe), otherwise malloc. The byte budget limits
//   *live* bytes; released blocks stay cached for reuse until the cache
//   itself would exceed the budget, at which point they are freed.
// - mcv_meshbuf_release: verify header magic + canaries, route to the owning
//   pool (stored in the header) and return the block to its class free list.
// - mcv_pool_destroy: frees every cached block; aborts in debug builds if
//   buffers are still live or a canary was smashed.
//
// Block layout: [BlockHeader][8B canary][payload][8B canary]

#include "mcv.h"

#include <atomic>
#include <new>
#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace {

constexpr uint32_t kHeaderMagic = 0x4D435631u; /* "MCV1" */
[[maybe_unused]] constexpr uint32_t kFreeMagic = 0x46724545u; /* "FrEE",仅 MCV_DEBUG 引用 */
constexpr uint8_t kCanaryByte = 0xC7;
constexpr size_t kCanarySize = 8;
constexpr int kNumClasses = 5;
/* Payload capacity per class: 16 KiB .. 4 MiB */
constexpr uint64_t kClassBytes[kNumClasses] = {
    16384, 65536, 262144, 1048576, 4194304};

struct BlockHeader {
    uint32_t magic;
    uint32_t cls;
    uint64_t payload_size;
    struct Pool* owner;
};

/* Intrusive LIFO node living in the block payload. */
struct FreeNode {
    BlockHeader* next;
};

struct ClassStack {
    std::atomic<BlockHeader*> top{nullptr};
};

struct Pool {
    std::atomic<uint64_t> budget;
    std::atomic<uint64_t> bytes_live;
    std::atomic<uint64_t> bytes_cached;
    std::atomic<uint32_t> live_buffers;
    ClassStack stacks[kNumClasses];
};

thread_local Pool* g_active_pool = nullptr;

size_t block_total_size(uint32_t cls) {
    return sizeof(BlockHeader) + kCanarySize + kClassBytes[cls] + kCanarySize;
}

uint8_t* block_payload(BlockHeader* h) {
    uint8_t* base = reinterpret_cast<uint8_t*>(h);
    return base + sizeof(BlockHeader) + kCanarySize;
}

uint8_t* block_lead_canary(BlockHeader* h) {
    uint8_t* base = reinterpret_cast<uint8_t*>(h);
    return base + sizeof(BlockHeader);
}

uint8_t* block_tail_canary(BlockHeader* h) {
    uint8_t* base = reinterpret_cast<uint8_t*>(h);
    return base + block_total_size(h->cls) - kCanarySize;
}

void fill_canaries(BlockHeader* h) {
    std::memset(block_lead_canary(h), kCanaryByte, kCanarySize);
    std::memset(block_tail_canary(h), kCanaryByte, kCanarySize);
}

[[maybe_unused]] bool canaries_intact(BlockHeader* h) {  // release 下仅 #if MCV_DEBUG 引用
    uint8_t lead[kCanarySize];
    uint8_t tail[kCanarySize];
    std::memcpy(lead, block_lead_canary(h), kCanarySize);
    std::memcpy(tail, block_tail_canary(h), kCanarySize);
    for (size_t i = 0; i < kCanarySize; ++i) {
        if (lead[i] != kCanaryByte || tail[i] != kCanaryByte) {
            return false;
        }
    }
    return true;
}

int select_class(uint64_t needed) {
    for (int c = 0; c < kNumClasses; ++c) {
        if (kClassBytes[c] >= needed) {
            return c;
        }
    }
    return -1;
}

BlockHeader* pop_from_stack(Pool* p, int cls) {
    ClassStack& s = p->stacks[cls];
    BlockHeader* head = s.top.load(std::memory_order_acquire);
    for (;;) {
        if (head == nullptr) {
            return nullptr;
        }
        FreeNode* node = reinterpret_cast<FreeNode*>(block_payload(head));
        BlockHeader* next = node->next;
        if (s.top.compare_exchange_weak(head, next, std::memory_order_acq_rel,
                                        std::memory_order_acquire)) {
            return head;
        }
        /* head was reloaded by CAS */
    }
}

void push_to_stack(Pool* p, int cls, BlockHeader* h) {
    ClassStack& s = p->stacks[cls];
    FreeNode* node = reinterpret_cast<FreeNode*>(block_payload(h));
    BlockHeader* head = s.top.load(std::memory_order_relaxed);
    for (;;) {
        node->next = head;
        if (s.top.compare_exchange_weak(head, h, std::memory_order_acq_rel,
                                        std::memory_order_relaxed)) {
            return;
        }
    }
}

}  // namespace

extern "C" {

uint32_t mcv_api_version(void) { return MCV_ABI_VERSION; }

void mcv_set_active_pool(McvPool* pool) {
    g_active_pool = reinterpret_cast<Pool*>(pool);
}

/* Internal helper for mcv_mesh_build (mesher.cpp): exposes the thread-local
 * active pool so the mesher can acquire its output buffer. Not part of the
 * public mcv.h surface. */
McvPool* mcv_active_pool(void) {
    return reinterpret_cast<McvPool*>(g_active_pool);
}

McvPool* mcv_pool_create(uint64_t budget_bytes) {
    Pool* p = new (std::nothrow) Pool();
    if (p == nullptr) {
        return nullptr;
    }
    p->budget.store(budget_bytes ? budget_bytes : UINT64_MAX,
                    std::memory_order_relaxed);
    p->bytes_live.store(0, std::memory_order_relaxed);
    p->bytes_cached.store(0, std::memory_order_relaxed);
    p->live_buffers.store(0, std::memory_order_relaxed);
    return reinterpret_cast<McvPool*>(p);
}

void mcv_pool_destroy(McvPool* opaque) {
    Pool* p = reinterpret_cast<Pool*>(opaque);
    if (p == nullptr) {
        return;
    }
#if defined(MCV_DEBUG)
    if (p->live_buffers.load(std::memory_order_relaxed) != 0) {
        std::fprintf(stderr, "mcv mempool fatal: destroy with live buffers\n");
        std::abort();
    }
#endif
    for (int c = 0; c < kNumClasses; ++c) {
        for (;;) {
            BlockHeader* h = pop_from_stack(p, c);
            if (h == nullptr) {
                break;
            }
            std::free(h);
        }
    }
    if (g_active_pool == p) {
        g_active_pool = nullptr;
    }
    delete p;
}

int32_t mcv_meshbuf_acquire(McvPool* opaque, uint32_t vertex_cap,
                            uint32_t index_cap, McvMeshBuffer* out) {
    Pool* p = reinterpret_cast<Pool*>(opaque);
    if (p == nullptr || out == nullptr) {
        return MCV_ERR_NULL_ARG;
    }
    if (vertex_cap == 0 && index_cap == 0) {
        return MCV_ERR_BAD_ARG;
    }
    uint64_t needed = static_cast<uint64_t>(vertex_cap) * 24 +
                      static_cast<uint64_t>(index_cap) * 4;
    int cls = select_class(needed);
    if (cls < 0) {
        return MCV_ERR_OOM;
    }
    size_t total = block_total_size(static_cast<uint32_t>(cls));

    BlockHeader* h = pop_from_stack(p, cls);
    if (h != nullptr) {
#if defined(MCV_DEBUG)
        if (h->magic != kFreeMagic || !canaries_intact(h)) {
            return MCV_ERR_CANARY_CORRUPT;
        }
#endif
        p->bytes_cached.fetch_sub(total, std::memory_order_relaxed);
    } else {
        if (p->bytes_live.load(std::memory_order_relaxed) + total >
            p->budget.load(std::memory_order_relaxed)) {
            return MCV_ERR_OOM;
        }
        h = static_cast<BlockHeader*>(std::malloc(total));
        if (h == nullptr) {
            return MCV_ERR_OOM;
        }
        h->magic = kHeaderMagic;
        h->cls = static_cast<uint32_t>(cls);
        h->payload_size = kClassBytes[cls];
        h->owner = p;
        fill_canaries(h);
    }

    p->bytes_live.fetch_add(total, std::memory_order_relaxed);
    p->live_buffers.fetch_add(1, std::memory_order_relaxed);
    h->magic = kHeaderMagic;

    uint8_t* payload = block_payload(h);
    out->vertex_data = payload;
    out->indices =
        reinterpret_cast<uint32_t*>(payload + static_cast<uint64_t>(vertex_cap) * 24);
    out->vertex_count = 0;
    out->index_count = 0;
    out->vertex_cap = vertex_cap;
    out->index_cap = index_cap;
    out->pool_class = static_cast<uint32_t>(cls);
    return MCV_OK;
}

int32_t mcv_meshbuf_release(McvMeshBuffer* buf) {
    if (buf == nullptr || buf->vertex_data == nullptr) {
        return MCV_ERR_NULL_ARG;
    }
    BlockHeader* h = reinterpret_cast<BlockHeader*>(buf->vertex_data -
                                                    kCanarySize -
                                                    sizeof(BlockHeader));
    uint32_t cls = h->cls;
    if (cls >= kNumClasses) {
        return MCV_ERR_BAD_ARG;
    }
    Pool* p = reinterpret_cast<Pool*>(h->owner);
    if (p == nullptr) {
        return MCV_ERR_BAD_ARG;
    }
#if defined(MCV_DEBUG)
    if (h->magic == kFreeMagic) {
        return MCV_ERR_DOUBLE_FREE;
    }
    if (h->magic != kHeaderMagic || !canaries_intact(h)) {
        return MCV_ERR_CANARY_CORRUPT;
    }
    h->magic = kFreeMagic;
#endif
    size_t total = block_total_size(cls);

    p->bytes_live.fetch_sub(total, std::memory_order_relaxed);
    p->live_buffers.fetch_sub(1, std::memory_order_relaxed);

    uint64_t cached = p->bytes_cached.load(std::memory_order_relaxed);
    if (cached + total <= p->budget.load(std::memory_order_relaxed)) {
        push_to_stack(p, static_cast<int>(cls), h);
        p->bytes_cached.fetch_add(total, std::memory_order_relaxed);
    } else {
        std::free(h);
    }
    return MCV_OK;
}

uint32_t mcv_pool_stats(McvPool* opaque, uint32_t* out_live_buffers,
                        uint64_t* out_bytes_live) {
    Pool* p = reinterpret_cast<Pool*>(opaque);
    if (p == nullptr) {
        return 0;
    }
    if (out_live_buffers != nullptr) {
        *out_live_buffers = p->live_buffers.load(std::memory_order_relaxed);
    }
    if (out_bytes_live != nullptr) {
        *out_bytes_live = p->bytes_live.load(std::memory_order_relaxed);
    }
    return 1;
}

}  // extern "C"
