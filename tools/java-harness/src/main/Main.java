// HARNESS-PATCH: n/a - harness bootstrap entrypoint (not a vanilla file)
// main.Main = black-box worldgen dumper for Minecraft 26.1 parity.
//
// Pipeline: bootstrap -> datapack registries (RegistryDataLoader over JSON)
//   -> RandomState.create(overworld, seed) -> NoiseChunk fill loop (HarnessFill,
//   verbatim mirror of NoiseBasedChunkGenerator.doFill) -> per-voxel BlockState ids.
//
// Usage (from /root/java-harness after ./build.sh):
//   java -cp "build:libs/*" Main --seed 123 --chunk 0,0 [-o out.bin] [--y-min -64 --y-max 320]
//   java -cp "build:libs/*" Main --seed 123 --density y,X,Z
//   [--data-root DIR] datapack root containing minecraft/worldgen (local precondition)
//
// Output "JGH1" binary (see tools/java_dupe.py):
//   magic JGH1 | i32 idCount | i64 seed | i32 chunkX | i32 chunkZ | i32 yMin | i32 yMax(exclusive)
//   | idCount x (u16 len + utf8 blockstate name) | (16*16*(yMax-yMin)) x u16 LE voxel ids,
//   y,z,x order (y outermost), id 0 = minecraft:air.
// Sidecar JSON: out.bin.json (meta + id->name table).

import java.io.DataOutputStream;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.PathPackResources;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.core.RegistryAccess;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.DensityFunction;
import net.minecraft.world.level.levelgen.HarnessFill;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.block.Blocks;

public class Main {
   private static final java.util.Set<net.minecraft.resources.ResourceKey<? extends net.minecraft.core.Registry<?>>> LOADED_REGISTRIES =
      java.util.Set.of(Registries.NOISE_SETTINGS, Registries.NOISE, Registries.DENSITY_FUNCTION);

   public static void main(String[] args) throws Exception {
      long seed = 0;
      int chunkX = 0;
      int chunkZ = 0;
      int yMin = Integer.MIN_VALUE;
      int yMax = Integer.MIN_VALUE;
      String out = null;
      String densityArg = null;
      java.util.List<String> probeNoiseArgs = new java.util.ArrayList<>();
      java.util.List<String> probePrelimArgs = new java.util.ArrayList<>();
      Path dataRoot = null;
      for (int i = 0; i < args.length; i++) {
         switch (args[i]) {
            case "--seed" -> seed = Long.parseLong(args[++i]);
            case "--chunk" -> {
               String[] p = args[++i].split(",");
               chunkX = Integer.parseInt(p[0]);
               chunkZ = Integer.parseInt(p[1]);
            }
            case "--y-min" -> yMin = Integer.parseInt(args[++i]);
            case "--y-max" -> yMax = Integer.parseInt(args[++i]);
            case "-o" -> out = args[++i];
            case "--density" -> densityArg = args[++i];
            case "--probe-noise" -> probeNoiseArgs.add(args[++i]);
            case "--probe-prelim" -> probePrelimArgs.add(args[++i]);
            case "--data-root" -> dataRoot = Path.of(args[++i]);
            default -> {
               System.err.println("unknown arg: " + args[i]);
               System.exit(2);
            }
         }
      }
      if (dataRoot == null) {
         dataRoot = Path.of(System.getProperty("harness.data", "/root/mc-ref/src-26.1/data"));
      }
      // PathPackResources expects <root>/data/<namespace>/... - accept either the
      // parent dir or the data dir itself and normalize to the parent.
      if (dataRoot.getFileName() != null && dataRoot.getFileName().toString().equals("data")) {
         dataRoot = dataRoot.getParent();
      }
      if (!Files.isDirectory(dataRoot.resolve("data/minecraft/worldgen"))) {
         System.err.println("data root not found or lacks data/minecraft/worldgen: " + dataRoot
            + "  (pass --data-root or -Dharness.data=...; see README)");
         System.exit(2);
      }

      SharedConstants.tryDetectVersion();
      Bootstrap.bootStrap();

      RegistryAccess.Frozen frozen = loadRegistries(dataRoot);
      NoiseGeneratorSettings settings = frozen.lookupOrThrow(Registries.NOISE_SETTINGS)
         .getOrThrow(NoiseGeneratorSettings.OVERWORLD).value();
      RandomState randomState = RandomState.create(frozen, NoiseGeneratorSettings.OVERWORLD, seed);
      NoiseSettings vanillaNoiseSettings = settings.noiseSettings();
      System.out.printf("settings=%s seaLevel=%d noiseY=[%d,%d)%n",
         NoiseGeneratorSettings.OVERWORLD.identifier(), settings.seaLevel(),
         vanillaNoiseSettings.minY(), vanillaNoiseSettings.minY() + vanillaNoiseSettings.height());

      if (!probeNoiseArgs.isEmpty()) {
         // repeated flags: --probe-noise name,y,x,z --probe-noise name2,y2,x2,z2 ...
         for (String spec : probeNoiseArgs) {
            String[] p = spec.split(",");
            String name = p[0];
            int y = Integer.parseInt(p[1]);
            int x = Integer.parseInt(p[2]);
            int z = Integer.parseInt(p[3]);
            var key = ResourceKey.create(Registries.NOISE, net.minecraft.resources.Identifier.withDefaultNamespace(name));
            net.minecraft.world.level.levelgen.synth.NormalNoise noise = randomState.getOrCreateNoise(key);
            double v = noise.getValue(x, y, z);
            System.out.printf("noise %s y=%d x=%d z=%d decimal=%.17g bits=0x%016x%n",
               name, y, x, z, v, Double.doubleToLongBits(v));
         }
         return;
      }

      if (!probePrelimArgs.isEmpty()) {
         // repeated flags: --probe-prelim x,z ...（FindTopSurface 只消费 blockX/Z）
         for (String spec : probePrelimArgs) {
            String[] p = spec.split(",");
            int x = Integer.parseInt(p[0]);
            int z = Integer.parseInt(p[1]);
            double v = randomState.router().preliminarySurfaceLevel()
               .compute(new DensityFunction.SinglePointContext(x, 0, z));
            System.out.printf("prelim x=%d z=%d decimal=%.17g floor8=%d%n",
               x, z, v, (long) Math.floor(v / 8.0) * 8);
         }
         return;
      }

      if (densityArg != null) {
         String[] p = densityArg.split(",");
         int y = Integer.parseInt(p[0]);
         int x = Integer.parseInt(p[1]);
         int z = Integer.parseInt(p[2]);
         double v = randomState.router().finalDensity()
            .compute(new DensityFunction.SinglePointContext(x, y, z));
         // decimal for humans, hex fp bits for bit-exact python comparison
         System.out.printf("density y=%d x=%d z=%d decimal=%.17g bits=0x%016x%n",
            y, x, z, v, Double.doubleToLongBits(v));
         if (out != null) {
            Files.writeString(Path.of(out), String.format("%.17g 0x%016x%n", v, Double.doubleToLongBits(v)));
         }
         return;
      }

      if (out == null) {
         System.err.println("nothing to do: give -o out.bin (voxel dump) or --density y,X,Z");
         System.exit(2);
      }

      NoiseSettings clamped = vanillaNoiseSettings
         .clampToHeightAccessor(LevelHeightAccessor.create(vanillaNoiseSettings.minY(), vanillaNoiseSettings.height()));
      NoiseChunk noiseChunk = new NoiseChunk(
         16 / clamped.getCellWidth(),
         randomState,
         chunkX * 16,
         chunkZ * 16,
         clamped,
         Beardifier.EMPTY,
         settings,
         createFluidPicker(settings),
         Blender.empty()
      );

      int fillMinY = clamped.minY();
      int fillMaxY = clamped.minY() + clamped.height();
      int aMin = yMin == Integer.MIN_VALUE ? fillMinY : yMin;
      int aMax = yMax == Integer.MIN_VALUE ? fillMaxY : yMax;
      if (aMin < fillMinY || aMax > fillMaxY || aMin >= aMax) {
         System.err.printf("y range [%d,%d) outside noise fill span [%d,%d)%n", aMin, aMax, fillMinY, fillMaxY);
         System.exit(2);
      }
      int span = aMax - aMin;
      short[] voxels = new short[16 * 16 * span];
      Map<BlockState, Short> idTable = new HashMap<>();
      List<String> names = new ArrayList<>();

      HarnessFill.fill(noiseChunk, settings, clamped, new ChunkPos(chunkX, chunkZ), fillMinY, voxels, idTable, names, aMin, aMax);

      writeBin(Path.of(out), seed, chunkX, chunkZ, aMin, aMax, voxels, names);
      Map<String, Object> sidecar = new java.util.LinkedHashMap<>();
      sidecar.put("format", "jgh1");
      sidecar.put("mc_version", SharedConstants.getCurrentVersion().id());
      sidecar.put("seed", seed);
      sidecar.put("chunk", List.of(chunkX, chunkZ));
      sidecar.put("y_min", aMin);
      sidecar.put("y_max", aMax);
      sidecar.put("order", "y,z,x (y outermost, x innermost)");
      sidecar.put("stage", "noise");
      sidecar.put("beardifier", "EMPTY (no structure beards)");
      sidecar.put("ids", names);
      String json = new com.google.gson.GsonBuilder().disableHtmlEscaping().create().toJson(sidecar);
      Files.writeString(Path.of(out + ".json"), json + "\n");
      System.out.printf("wrote %s (%d voxels, %d distinct states) + sidecar %s%n",
         out, voxels.length, names.size(), out + ".json");
   }

   // Verbatim mirror of NoiseBasedChunkGenerator.createFluidPicker (private there).
   private static Aquifer.FluidPicker createFluidPicker(final NoiseGeneratorSettings st) {
      Aquifer.FluidStatus lavaStatus = new Aquifer.FluidStatus(-54, Blocks.LAVA.defaultBlockState());
      int seaLevel = st.seaLevel();
      Aquifer.FluidStatus seaStatus = new Aquifer.FluidStatus(seaLevel, st.defaultFluid());
      Aquifer.FluidStatus emptyStatus = new Aquifer.FluidStatus(DimensionType.MIN_Y * 2, Blocks.AIR.defaultBlockState());
      return (x, y, z) -> {
         if (SharedConstants.DEBUG_DISABLE_FLUID_GENERATION) {
            return emptyStatus;
         } else {
            return y < Math.min(-54, seaLevel) ? lavaStatus : seaStatus;
         }
      };
   }

   private static RegistryAccess.Frozen loadRegistries(Path dataRoot) {
      ResourceManager resourceManager = new MultiPackResourceManager(
         PackType.SERVER_DATA,
         List.of(new PathPackResources(
            new net.minecraft.server.packs.PackLocationInfo("harness", Component_forHarness(), PackSource.DEFAULT, java.util.Optional.empty()),
            dataRoot))
      );
      List<RegistryDataLoader.RegistryData<?>> subset = new ArrayList<>();
      for (RegistryDataLoader.RegistryData<?> d : RegistryDataLoader.WORLDGEN_REGISTRIES) {
         if (LOADED_REGISTRIES.contains(d.key())) {
            subset.add(d);
         }
      }
      return RegistryDataLoader.load(resourceManager, List.of(), subset, Runnable::run).join();
   }

   private static net.minecraft.network.chat.Component Component_forHarness() {
      return net.minecraft.network.chat.Component.literal("java-harness datapacks");
   }

   private static void writeBin(Path out, long seed, int chunkX, int chunkZ, int aMin, int aMax, short[] voxels, List<String> names) throws Exception {
      try (DataOutputStream dos = new DataOutputStream(new FileOutputStream(out.toFile()))) {
         dos.writeBytes("JGH1");
         dos.writeInt(names.size());
         dos.writeLong(seed);
         dos.writeInt(chunkX);
         dos.writeInt(chunkZ);
         dos.writeInt(aMin);
         dos.writeInt(aMax);
         for (String n : names) {
            byte[] b = n.getBytes(StandardCharsets.UTF_8);
            dos.writeShort(b.length);
            dos.write(b);
         }
         for (short v : voxels) {
            dos.writeShort(v);
         }
      }
   }
}
