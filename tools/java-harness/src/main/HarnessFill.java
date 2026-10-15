// HARNESS-PATCH: n/a - harness-owned bridge class (NOT vanilla). Lives in this
// package so it can reach NoiseChunk's protected interpolation accessors
// (getInterpolatedState / cellWidth / cellHeight), exactly what
// NoiseBasedChunkGenerator.doFill uses. The fill loop below is a verbatim
// shape-mirror of NoiseBasedChunkGenerator.doFill with the section writes
// replaced by a flat short[] voxel id sink. Beardifier is injected by the
// caller (harness passes EMPTY = no structure beards).
package net.minecraft.world.level.levelgen;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.util.Mth;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

public final class HarnessFill {
   private HarnessFill() {
   }

   /** Mirrors NoiseBasedChunkGenerator.doFill: iterates cells/voxels in vanilla
    *  order, sinks interned state ids into voxels[] (0 = air, interned first). */
   public static void fill(
      final NoiseChunk noiseChunk,
      final NoiseGeneratorSettings settings,
      final NoiseSettings noiseSettings,
      final ChunkPos chunkPos,
      final int fillMinY,
      final short[] voxels,
      final Map<BlockState, Short> idTable,
      final List<String> names,
      final int aMin,
      final int aMax
   ) {
      intern(Blocks.AIR.defaultBlockState(), idTable, names);
      BlockState defaultBlock = settings.defaultBlock();
      noiseChunk.initializeForFirstCellX();
      int cellWidth = noiseChunk.cellWidth();
      int cellHeight = noiseChunk.cellHeight();
      int cellCountY = Mth.floorDiv(noiseSettings.height(), cellHeight);
      int cellMinY = Mth.floorDiv(fillMinY, cellHeight);
      int chunkStartBlockX = chunkPos.getMinBlockX();
      int chunkStartBlockZ = chunkPos.getMinBlockZ();
      int cellCountXZ = 16 / cellWidth;

      for (int cellXIndex = 0; cellXIndex < cellCountXZ; cellXIndex++) {
         noiseChunk.advanceCellX(cellXIndex);

         for (int cellZIndex = 0; cellZIndex < cellCountXZ; cellZIndex++) {
            for (int cellYIndex = cellCountY - 1; cellYIndex >= 0; cellYIndex--) {
               noiseChunk.selectCellYZ(cellYIndex, cellZIndex);

               for (int yInCell = cellHeight - 1; yInCell >= 0; yInCell--) {
                  int posY = (cellMinY + cellYIndex) * cellHeight + yInCell;
                  double factorY = (double)yInCell / cellHeight;
                  noiseChunk.updateForY(posY, factorY);

                  for (int xInCell = 0; xInCell < cellWidth; xInCell++) {
                     int posX = chunkStartBlockX + cellXIndex * cellWidth + xInCell;
                     double factorX = (double)xInCell / cellWidth;
                     noiseChunk.updateForX(posX, factorX);

                     for (int zInCell = 0; zInCell < cellWidth; zInCell++) {
                        int posZ = chunkStartBlockZ + cellZIndex * cellWidth + zInCell;
                        double factorZ = (double)zInCell / cellWidth;
                        noiseChunk.updateForZ(posZ, factorZ);
                        BlockState state = noiseChunk.getInterpolatedState();
                        if (state == null) {
                           state = defaultBlock;
                        }
                        if (posY < aMin || posY >= aMax || state.isAir()) {
                           continue; // voxel id stays 0 = air
                        }
                        voxels[idx(posX & 15, posY - aMin, posZ & 15)] = intern(state, idTable, names);
                     }
                  }
               }
            }
         }
      }
      noiseChunk.stopInterpolation();
   }

   private static int idx(final int x, final int yOffset, final int z) {
      // y,z,x with y outermost, x innermost
      return (yOffset * 16 + z) * 16 + x;
   }

   private static short intern(final BlockState state, final Map<BlockState, Short> idTable, final List<String> names) {
      Short id = idTable.get(state);
      if (id == null) {
         id = (short)names.size();
         idTable.put(state, id);
         names.add(blockStateName(state));
      }
      return id;
   }

   public static String blockStateName(final BlockState state) {
      String name = net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
      List<String> parts = new java.util.ArrayList<>();
      for (Property<?> p : state.getProperties()) {
         parts.add(p.getName() + "=" + valueName(state, p));
      }
      java.util.Collections.sort(parts);
      return parts.isEmpty() ? name : name + "[" + String.join(",", parts) + "]";
   }

   private static <T extends Comparable<T>> String valueName(final BlockState state, final Property<T> p) {
      return p.getName(state.getValue(p));
   }
}
