// Fortnite Ring setup: converts the Fortnite content the mashup uses from the player's OWN Fortnite
// install into %LOCALAPPDATA%\FortniteRing\cache. Melty runs it before the first start (recipe.setup)
// and passes the Fortnite folder ({game:fortnite}). Nothing from Fortnite ships with the mod.
//
//   FortniteRingSetup.exe --fortnite "<Fortnite folder>" --out "<cache folder>" [--outfit CID_...] [--pickaxe Pickaxe_ID_...]
//   FortniteRingSetup.exe --fortnite "<Fortnite folder>" --list "<regex>"   (prints every matching package path; for fixing sheet queries)
//   FortniteRingSetup.exe --fortnite "<Fortnite folder>" --dump "<package path>"   (prints the package's exports as JSON)
//
// Steps: read the AES keys and type mappings for the installed build from fortnite-api.com, mount the
// paks with CUE4Parse, resolve every row of sheets/fortnite_assets.json (Generated.cs), export it, and
// write manifest.json (what was found where, what is missing). The DLL reads manifest.json at boot.
using System.Diagnostics;
using System.Text.Json;
using System.Text.RegularExpressions;
using CUE4Parse.Compression;
using CUE4Parse.Encryption.Aes;
using CUE4Parse.FileProvider;
using CUE4Parse.MappingsProvider.Usmap;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Sound;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.Core.Misc;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Versions;
using CUE4Parse_Conversion;
using CUE4Parse_Conversion.Options;
using CUE4Parse_Conversion.Sounds;
using CUE4Parse_Conversion.Textures;

namespace FortniteRing.Setup;

public static class Program
{
    const string DefaultOutfit = "CID_883_Athena_Commando_M_ChOneJonesy";
    const string DefaultPickaxe = "DefaultPickaxe";
    static readonly HttpClient Http = new() { Timeout = TimeSpan.FromMinutes(5) };
    static StreamWriter? _log;

    static void Log(string msg)
    {
        var line = $"[{DateTime.Now:HH:mm:ss}] {msg}";
        Console.WriteLine(line);
        _log?.WriteLine(line);
        _log?.Flush();
    }

    public static async Task<int> Main(string[] args)
    {
        string Arg(string name, string? fallback = null)
        {
            var i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : fallback ?? "";
        }
        var local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        var outDir = Arg("--out", Path.Combine(local, "FortniteRing", "cache"));
        var fortnite = Arg("--fortnite", Environment.GetEnvironmentVariable("FORTNITE_RING_FORTNITE") ?? "");
        var outfit = Arg("--outfit", Environment.GetEnvironmentVariable("FORTNITE_RING_OUTFIT") ?? DefaultOutfit);
        var pickaxe = Arg("--pickaxe", Environment.GetEnvironmentVariable("FORTNITE_RING_PICKAXE") ?? DefaultPickaxe);
        var list = Arg("--list");
        var dump = Arg("--dump");
        var inspecting = list.Length > 0 || dump.Length > 0; // inspection modes leave setup.log and the manifest alone
        Directory.CreateDirectory(outDir);
        if (!inspecting) _log = new StreamWriter(Path.Combine(outDir, "..", "setup.log"), append: false);
        Log($"Fortnite Ring setup 0.1.0: Fortnite at '{fortnite}', cache at '{outDir}'");
        try
        {
            return await Run(fortnite, outDir, outfit, pickaxe, list.Length > 0 ? list : null, dump.Length > 0 ? dump : null);
        }
        catch (Exception e)
        {
            Log($"FAILED: {e}");
            if (inspecting) return 1;
            await WriteManifest(outDir, "unknown", new(), Sheet.Assets.Where(a => a.Required).Select(a => a.Id).ToList(), e.Message);
            return 1;
        }
    }

    static async Task<int> Run(string fortnite, string outDir, string outfit, string pickaxe, string? list, string? dump)
    {
        var paks = Path.Combine(fortnite, "FortniteGame", "Content", "Paks");
        if (!Directory.Exists(paks) || Directory.GetFiles(paks, "*.utoc").Length == 0)
            throw new DirectoryNotFoundException($"No Fortnite game files at {paks}. Is Fortnite fully installed?");

        // the same build already converted? nothing to do
        using var aesDoc = JsonDocument.Parse(await Http.GetStringAsync("https://fortnite-api.com/v2/aes"));
        var aes = aesDoc.RootElement.GetProperty("data");
        var build = aes.GetProperty("build").GetString() ?? "unknown";
        var manifestPath = Path.Combine(outDir, "manifest.json");
        if (list == null && dump == null && File.Exists(manifestPath))
        {
            using var old = JsonDocument.Parse(await File.ReadAllTextAsync(manifestPath));
            if (old.RootElement.GetProperty("fortnite_build").GetString() == build &&
                old.RootElement.GetProperty("missing_required").GetArrayLength() == 0 &&
                old.RootElement.TryGetProperty("sheet_rows", out var rows) && rows.GetInt32() == Sheet.Assets.Length)
            {
                Log($"cache already matches {build}; nothing to do");
                return 0;
            }
        }

        var tools = Path.Combine(outDir, "..", "tools");
        Directory.CreateDirectory(tools);
        string? oodle = Path.Combine(tools, OodleHelper.OodleFileName);
        if (!File.Exists(oodle)) OodleHelper.DownloadOodleDll(ref oodle);
        OodleHelper.Initialize(oodle);
        var zlib = Path.Combine(tools, ZlibHelper.DllName);
        if (!File.Exists(zlib)) await ZlibHelper.DownloadDllAsync(zlib);
        ZlibHelper.Initialize(zlib);

        // type mappings for the build (unversioned properties), published by uedb.dev
        using var mapDoc = JsonDocument.Parse(await Http.GetStringAsync("https://uedb.dev/svc/api/v1/fortnite/mappings"));
        var mapVersion = mapDoc.RootElement.GetProperty("version").GetString();
        if (mapVersion != build) Log($"warning: mappings are for {mapVersion}, keys for {build}");
        var mapUrl = mapDoc.RootElement.GetProperty("mappings").GetProperty("Brotli").GetString()!;
        var mapFile = Path.Combine(tools, Path.GetFileName(new Uri(mapUrl).LocalPath));
        if (!File.Exists(mapFile)) await File.WriteAllBytesAsync(mapFile, await Http.GetByteArrayAsync(mapUrl));
        Log($"Fortnite {build}: keys and mappings fetched");

        var provider = new DefaultFileProvider(paks, SearchOption.TopDirectoryOnly, true, new VersionContainer(EGame.GAME_UE5_LATEST));
        provider.MappingsContainer = new FileUsmapTypeMappingsProvider(mapFile);
        provider.Initialize();
        var keys = new Dictionary<FGuid, FAesKey> { [new FGuid()] = new FAesKey(aes.GetProperty("mainKey").GetString()!) };
        foreach (var dk in aes.GetProperty("dynamicKeys").EnumerateArray())
            keys[new FGuid(dk.GetProperty("pakGuid").GetString()!)] = new FAesKey(dk.GetProperty("key").GetString()!);
        var mounted = provider.SubmitKeys(keys);
        provider.PostMount();
        Log($"mounted {mounted} archives, {provider.Files.Count} files");
        if (provider.Files.Count == 0)
            throw new InvalidOperationException("Fortnite's files could not be opened with the published keys (Fortnite may have just updated; try again later).");

        var paths = provider.Files.Keys.Where(k => k.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase)).ToList();
        if (list != null)
        {
            var lrx = new Regex(list, RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
            foreach (var p in paths.Where(p => lrx.IsMatch(Path.ChangeExtension(p, null))).OrderBy(p => p)) Console.WriteLine(p);
            return 0;
        }
        if (dump != null)
        {
            Console.WriteLine(Newtonsoft.Json.JsonConvert.SerializeObject(provider.LoadPackage(dump).GetExports(), Newtonsoft.Json.Formatting.Indented));
            return 0;
        }
        var found = new Dictionary<string, string>();
        var missing = new List<string>();
        foreach (var row in Sheet.Assets)
        {
            try
            {
                var source = Resolve(provider, paths, row, outfit, pickaxe);
                if (source.Count == 0)
                {
                    Log($"  {row.Id}: not found ({row.Query})");
                    if (row.Kind == "sound" && row.Id == "snd_none") { WriteSilence(Path.Combine(outDir, row.Out)); found[row.Id] = "(silence)"; }
                    else if (row.Required) missing.Add(row.Id);
                    continue;
                }
                for (var i = 0; i < source.Count; i++)
                {
                    var target = Path.Combine(outDir, i == 0 ? row.Out : InsertSuffix(row.Out, $"_{i}"));
                    Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                    await Export(source[i], row.Kind, target, tools);
                }
                found[row.Id] = string.Join(" | ", source.Select(s => s.GetPathName()));
                Log($"  {row.Id}: {found[row.Id]}");
            }
            catch (Exception e)
            {
                Log($"  {row.Id}: export failed: {e.Message}");
                if (row.Required) missing.Add(row.Id);
            }
        }
        await WriteManifest(outDir, build, found, missing, null);
        Log($"done: {found.Count} found, {missing.Count} required missing");
        return missing.Count == 0 ? 0 : 2;
    }

    static bool IsSfnt(byte[] b, int at) =>
        b.Length >= at + 4 && ((b[at] == 0 && b[at + 1] == 1 && b[at + 2] == 0 && b[at + 3] == 0) ||
                               "OTTO,true,ttcf".Split(',').Any(t => b[at] == t[0] && b[at + 1] == t[1] && b[at + 2] == t[2] && b[at + 3] == t[3]));

    /// Cooked font bytes (.ufont bulk data) start with an int32 byte count before the TrueType/OpenType file
    /// (seen on Fortnite 42.30); the DLL's font loader needs the bare file.
    static byte[] StripFontPrefix(byte[] data)
    {
        if (IsSfnt(data, 0)) return data;
        if (IsSfnt(data, 4))
        {
            var n = BitConverter.ToInt32(data, 0);
            return data.AsSpan(4, n > 0 && n <= data.Length - 4 ? n : data.Length - 4).ToArray();
        }
        throw new InvalidDataException("font data is not a TrueType/OpenType file");
    }

    static string InsertSuffix(string path, string suffix) =>
        Path.Combine(Path.GetDirectoryName(path) ?? "", Path.GetFileNameWithoutExtension(path) + suffix + Path.GetExtension(path));

    /// Finds the object(s) a sheet row points at.
    static List<UObject> Resolve(IFileProvider provider, List<string> paths, AssetRow row, string outfit, string pickaxe)
    {
        var q = row.Query;
        var cut = q.IndexOf("  (", StringComparison.Ordinal);
        if (cut > 0) q = q[..cut];
        if (q.StartsWith("outfit:")) return ResolveOutfit(provider, paths, outfit);
        if (q.StartsWith("pickaxe:")) return ResolvePickaxe(provider, paths, pickaxe);
        string? follow = null;
        var arrow = q.IndexOf(" -> ", StringComparison.Ordinal);
        if (arrow > 0) { follow = q[(arrow + 4)..].Trim(); q = q[..arrow]; }
        var rx = new Regex(q.Trim(), RegexOptions.CultureInvariant);
        // shortest matching path first: the base asset rather than a variant
        foreach (var p in paths.Where(p => rx.IsMatch(Path.ChangeExtension(p, null))).OrderBy(p => p.Length).Take(8))
        {
            var obj = provider.LoadPackage(p).GetExports().FirstOrDefault(e => KindMatches(e, row.Kind) || follow != null);
            if (obj == null) continue;
            if (follow == null) return [obj];
            foreach (var prop in follow.Split('|'))
            {
                var next = FollowProperty(obj, prop.Trim());
                if (next != null && KindMatches(next, row.Kind)) return [next];
            }
        }
        return [];
    }

    static bool KindMatches(UObject o, string kind) => kind switch
    {
        "skeletal_mesh" => o is CUE4Parse.UE4.Assets.Exports.SkeletalMesh.USkeletalMesh,
        "static_mesh" => o is CUE4Parse.UE4.Assets.Exports.StaticMesh.UStaticMesh || o is CUE4Parse.UE4.Assets.Exports.SkeletalMesh.USkeletalMesh,
        "anim" => o is CUE4Parse.UE4.Assets.Exports.Animation.UAnimSequenceBase,
        "texture" => o is UTexture2D,
        "sound" => o is USoundWave || o.ExportType.Contains("AkAudioEvent") || o.ExportType.Contains("AkMediaAsset"),
        "font" => o.ExportType.Contains("FontFace"),
        _ => false,
    };

    static UObject? FollowProperty(UObject o, string prop)
    {
        if (o.TryGetValue(out FSoftObjectPath soft, prop)) return soft.TryLoad(out var u) ? u : null;
        if (o.TryGetValue(out FPackageIndex idx, prop)) return idx.TryLoad(out var u) ? u : null;
        if (o.TryGetValue(out UObject obj, prop)) return obj;
        // current item definitions keep icons and pickup meshes in DataList (instanced structs):
        // find the first soft reference named `prop` anywhere in the object and load it by path
        var json = Newtonsoft.Json.Linq.JToken.FromObject(o, Newtonsoft.Json.JsonSerializer.Create());
        foreach (var p in json.SelectTokens("$.." + prop + ".AssetPathName"))
        {
            var path = (string?)p;
            if (!string.IsNullOrEmpty(path) && o.Owner?.Provider?.TryLoadPackageObject(path, out var loaded) == true) return loaded;
        }
        return null;
    }

    static List<UObject> ResolveOutfit(IFileProvider provider, List<string> paths, string cid)
    {
        var path = paths.FirstOrDefault(p => Path.GetFileNameWithoutExtension(p).Equals(cid, StringComparison.OrdinalIgnoreCase));
        if (path == null)
        {
            Log($"  outfit {cid} is not in this install (cosmetics stream on demand); using the default outfit");
            path = paths.Where(p => Regex.IsMatch(p, @"/CID_DefaultOutfit|/CID_001_Athena_Commando_F_Default", RegexOptions.IgnoreCase)).OrderBy(p => p.Length).FirstOrDefault();
            if (path == null) return [];
        }
        var character = provider.LoadPackage(path).GetExports().First();
        var parts = new List<UObject>();
        if (character.TryGetValue(out FSoftObjectPath[] baseParts, "BaseCharacterParts"))
            foreach (var p in baseParts)
                if (p.TryLoad(out var part) && part.TryGetValue(out FSoftObjectPath mesh, "SkeletalMesh") && mesh.TryLoad(out var m)) parts.Add(m);
        if (parts.Count == 0 && character.TryGetValue(out FSoftObjectPath hero, "HeroDefinition") && hero.TryLoad(out var heroDef)
            && heroDef.TryGetValue(out FSoftObjectPath[] specs, "Specializations"))
            foreach (var s in specs)
                if (s.TryLoad(out var spec) && spec.TryGetValue(out FSoftObjectPath[] cps, "CharacterParts"))
                    foreach (var cp in cps)
                        if (cp.TryLoad(out var part) && part.TryGetValue(out FSoftObjectPath mesh, "SkeletalMesh") && mesh.TryLoad(out var m)) parts.Add(m);
        return parts;
    }

    static List<UObject> ResolvePickaxe(IFileProvider provider, List<string> paths, string id)
    {
        var path = paths.Where(p => Path.GetFileNameWithoutExtension(p).Contains(id, StringComparison.OrdinalIgnoreCase)).OrderBy(p => p.Length).FirstOrDefault();
        if (path == null) return [];
        var item = provider.LoadPackage(path).GetExports().First();
        var weapon = FollowProperty(item, "WeaponDefinition") ?? item;
        foreach (var prop in new[] { "WeaponMeshOverride", "PickupStaticMesh", "PickupSkeletalMesh" })
            if (FollowProperty(weapon, prop) is { } mesh) return [mesh];
        return [];
    }

    static async Task Export(UObject obj, string kind, string target, string tools)
    {
        switch (kind)
        {
            case "texture":
                var tex = ((UTexture)obj).Decode(1024) ?? throw new InvalidDataException("texture decode failed");
                await File.WriteAllBytesAsync(target, tex.Encode(ETextureFormat.Png, false, out _));
                break;
            case "font":
                // a FontFace keeps the TTF inline, or (cooked) in a .ufont file next to the asset
                var data = (obj as CUE4Parse.UE4.Assets.Exports.Engine.Font.UFontFace)?.FontFaceData?.Data;
                if (data == null || data.Length == 0)
                {
                    var ufont = obj.Owner?.Provider?.Files.Keys.FirstOrDefault(k =>
                        k.EndsWith(".ufont", StringComparison.OrdinalIgnoreCase) &&
                        Path.GetFileNameWithoutExtension(k).StartsWith(obj.Name, StringComparison.OrdinalIgnoreCase));
                    if (ufont != null) data = await obj.Owner!.Provider!.SaveAssetAsync(ufont);
                }
                if (data == null || data.Length == 0) throw new InvalidDataException("font data not found");
                data = StripFontPrefix(data);
                await File.WriteAllBytesAsync(target, data);
                break;
            case "sound":
                obj.Decode(true, out var fmt, out var audio);
                if (audio == null) throw new InvalidDataException($"sound decode failed ({fmt})");
                await WriteWav(audio, fmt, target, tools);
                break;
            default:
                var session = new ExportSession { MaxDegreeOfParallelism = 1 };
                session.Add(obj);
                var tmp = Path.Combine(Path.GetTempPath(), "FortniteRingExport", Guid.NewGuid().ToString("N"));
                var format = kind == "anim" ? EMeshFormat.ActorX : EMeshFormat.Gltf2;
                var results = await session.RunAsync(tmp, new ExportOptions(meshFormat: format, exportMaterials: true, exportMorphTargets: false));
                var file = results.SelectMany(r => r.DiskFilePaths ?? []).FirstOrDefault(f => f.EndsWith(kind == "anim" ? ".psa" : ".glb", StringComparison.OrdinalIgnoreCase))
                           ?? throw new InvalidDataException($"exporter wrote no {(kind == "anim" ? "psa" : "glb")}: {string.Join(", ", results.Select(r => r.Error?.Message))}");
                File.Copy(file, target, true);
                if (kind != "anim") KeepTextures(file, target, results.SelectMany(r => r.DiskFilePaths ?? []));
                try { Directory.Delete(tmp, true); } catch { }
                break;
        }
    }

    /// Copies a glTF's textures into "<name>_tex/" next to it and points the .glb at them. The exporter writes
    /// them as relative paths like "../../Textures/x.png", which would land outside the cache folder.
    static void KeepTextures(string glb, string target, IEnumerable<string> files)
    {
        var bytes = File.ReadAllBytes(glb);
        if (bytes.Length < 20 || BitConverter.ToUInt32(bytes, 0) != 0x46546C67) return; // "glTF"
        var jsonLen = (int)BitConverter.ToUInt32(bytes, 12);
        var json = System.Text.Json.Nodes.JsonNode.Parse(System.Text.Encoding.UTF8.GetString(bytes, 20, jsonLen))!;
        var pngs = files.Where(f => f.EndsWith(".png", StringComparison.OrdinalIgnoreCase)).ToDictionary(Path.GetFullPath, f => f, StringComparer.OrdinalIgnoreCase);
        var texDir = Path.GetFileNameWithoutExtension(target) + "_tex";
        var changed = false;
        foreach (var image in json["images"]?.AsArray() ?? [])
        {
            var uri = image?["uri"]?.GetValue<string>();
            if (uri == null || uri.StartsWith("data:")) continue;
            var src = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(glb)!, Uri.UnescapeDataString(uri)));
            if (!pngs.ContainsKey(src) && !File.Exists(src)) continue;
            var dst = Path.Combine(Path.GetDirectoryName(target)!, texDir, Path.GetFileName(src));
            Directory.CreateDirectory(Path.GetDirectoryName(dst)!);
            File.Copy(src, dst, true);
            image!["uri"] = texDir + "/" + Uri.EscapeDataString(Path.GetFileName(src));
            changed = true;
        }
        if (!changed) return;
        var newJson = System.Text.Encoding.UTF8.GetBytes(json.ToJsonString());
        var padded = (newJson.Length + 3) & ~3;
        var rest = bytes.AsSpan(20 + jsonLen).ToArray(); // BIN chunk, unchanged
        using var w = new BinaryWriter(File.Create(target));
        w.Write(0x46546C67u); w.Write(2u); w.Write((uint)(12 + 8 + padded + rest.Length));
        w.Write((uint)padded); w.Write(0x4E4F534Au); // "JSON"
        w.Write(newJson); for (var i = newJson.Length; i < padded; i++) w.Write((byte)' ');
        w.Write(rest);
    }

    /// vgmstream-cli (Wwise/Bink/ADPCM to WAV) from its official release, kept with the other downloaded tools.
    const string VgmstreamZip = "https://github.com/vgmstream/vgmstream/releases/download/r2117/vgmstream-win64.zip";

    static async Task<string> Vgmstream(string tools)
    {
        var bundled = Path.Combine(AppContext.BaseDirectory, "vgmstream", "vgmstream-cli.exe");
        if (File.Exists(bundled)) return bundled;
        var dir = Path.Combine(tools, "vgmstream");
        var exe = Path.Combine(dir, "vgmstream-cli.exe");
        if (File.Exists(exe)) return exe;
        Log("downloading vgmstream r2117");
        var zip = Path.Combine(tools, "vgmstream-win64.zip");
        await File.WriteAllBytesAsync(zip, await Http.GetByteArrayAsync(VgmstreamZip));
        System.IO.Compression.ZipFile.ExtractToDirectory(zip, dir, true);
        File.Delete(zip);
        return exe;
    }

    /// PCM/ADPCM decode to WAV directly; Wwise (wem) and Bink go through vgmstream-cli.
    static async Task WriteWav(byte[] audio, string fmt, string target, string tools)
    {
        if (audio.Length > 12 && audio[0] == 'R' && audio[1] == 'I' && audio[2] == 'F' && audio[3] == 'F' &&
            !fmt.Equals("wem", StringComparison.OrdinalIgnoreCase))
        {
            await File.WriteAllBytesAsync(target, audio);
            return;
        }
        // RAD Audio (UE 5.4+, most Fortnite sounds) is not in vgmstream. Its decoder needs Epic's RAD Audio SDK
        // (Unreal Engine source), so it is not downloaded here: a radadec.exe placed in tools/ is used if present.
        var rada = fmt.Equals("rada", StringComparison.OrdinalIgnoreCase);
        var radadec = Path.Combine(tools, "radadec.exe");
        if (rada && !File.Exists(radadec))
            throw new InvalidDataException($"RAD Audio sound: put a RADA decoder at {radadec} to convert it");
        var converter = rada ? radadec : await Vgmstream(tools);
        var args = rada ? "-i \"{0}\" -o \"{1}\"" : "-o \"{1}\" \"{0}\"";
        var src = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N") + "." + fmt.ToLowerInvariant());
        await File.WriteAllBytesAsync(src, audio);
        var p = Process.Start(new ProcessStartInfo(converter, string.Format(args, src, target)) { CreateNoWindow = true, UseShellExecute = false })!;
        await p.WaitForExitAsync();
        File.Delete(src);
        if (p.ExitCode != 0 || !File.Exists(target)) throw new InvalidDataException($"{Path.GetFileName(converter)} could not convert {fmt}");
    }

    static void WriteSilence(string target)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(target)!);
        using var w = new BinaryWriter(File.Create(target));
        int rate = 22050, samples = 2205;
        w.Write("RIFF"u8); w.Write(36 + samples * 2); w.Write("WAVE"u8);
        w.Write("fmt "u8); w.Write(16); w.Write((short)1); w.Write((short)1); w.Write(rate); w.Write(rate * 2); w.Write((short)2); w.Write((short)16);
        w.Write("data"u8); w.Write(samples * 2); w.Write(new byte[samples * 2]);
    }

    static async Task WriteManifest(string outDir, string build, Dictionary<string, string> found, List<string> missing, string? error)
    {
        var doc = new Dictionary<string, object?>
        {
            ["fortnite_build"] = build,
            ["sheet_rows"] = Sheet.Assets.Length,
            ["found"] = found.Keys.ToList(),
            ["paths"] = found,
            ["missing_required"] = missing,
            ["error"] = error,
        };
        await File.WriteAllTextAsync(Path.Combine(outDir, "manifest.json"), JsonSerializer.Serialize(doc, new JsonSerializerOptions { WriteIndented = true }));
    }
}
