using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using HarmonyLib;
using MegaCrit.Sts2.Core.Models;
using MegaCrit.Sts2.Core.Models.Powers;

#pragma warning disable CA1861 // Expected arrays stay beside the assertions they describe.

namespace SpireProfiler;

// Producers are ordinal-sorted by their compact JSON with the declared field order.
// Candidates are evidence for manual review; only the embedded baseline is authoritative.
internal static class ProducerInventoryFixtures
{
    private sealed record Inventory
    {
        public required int FormatVersion { get; init; }
        public required string GameVersion { get; init; }
        public required Producer[] Producers { get; init; }
    }

    private sealed record Producer
    {
        public required string Kind { get; init; }
        public required string DefinitionRoot { get; init; }
        public required string DeclaringType { get; init; }
        public required string CallingConvention { get; init; }
        public required string Method { get; init; }
        public required string[] ParameterTypes { get; init; }
        public required string ReturnType { get; init; }

        internal string Key => JsonSerializer.Serialize(this, KeyOptions);

        internal static Producer FromMethod(string kind, MethodInfo method)
        {
            if (method == null) throw new InvalidDataException("Missing direct producer helper");
            string name = method.Name;
            if (method.IsGenericMethod) name += "<" + string.Join(',', method.GetGenericArguments().Select(TypeName)) + ">";
            return new()
            {
                Kind = kind,
                DefinitionRoot = TypeName(method.GetBaseDefinition().DeclaringType),
                DeclaringType = TypeName(method.DeclaringType),
                CallingConvention = method.IsStatic ? "static" : "instance",
                Method = name,
                ParameterTypes = method.GetParameters().Select(parameter =>
                    (parameter.IsOut ? "out " : parameter.IsIn ? "in " : parameter.ParameterType.IsByRef ? "ref " : "") + TypeName(parameter.ParameterType)).ToArray(),
                ReturnType = TypeName(method.ReturnType),
            };
        }
    }

    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        WriteIndented = true,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
    };
    private static readonly JsonSerializerOptions KeyOptions = new(Options) { WriteIndented = false };

    internal static void Run(string directory, string gameVersion)
    {
        var targets = FlowCapture.ProducerTargets(typeof(AbstractModel).Assembly);
        var helpers = new[] { (typeof(PoisonPower), "Trigger"), (typeof(RollingBoulderPower), "DoDamage") }
            .SelectMany(pair => pair.Item1.GetMethods(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                .Where(method => method.Name == pair.Item2)).ToArray();
        var candidate = Snapshot(gameVersion, targets, helpers);
        string path = Path.Combine(directory, "producer-inventory.candidate.json");
        File.WriteAllText(path, JsonSerializer.Serialize(candidate, Options) + "\n", new UTF8Encoding(false));
        Console.WriteLine("PRODUCER INVENTORY CANDIDATE: " + path);
        var changes = Compare(ReadReviewed(), candidate);
        if (changes.Length != 0)
            throw new InvalidDataException("Reviewed producer inventory differs; review the retained candidate before editing tests/producer-inventory.json:\n" + string.Join('\n', changes));

        // Equal counts cannot prove identity: replace a signature within the same root.
        var first = new Producer
        {
            Kind = "discovered",
            DefinitionRoot = "Game.Root",
            DeclaringType = "Game.Model",
            CallingConvention = "instance",
            Method = "Apply",
            ParameterTypes = new[] { "System.Int32" },
            ReturnType = "System.Void",
        };
        var replacement = first with { ParameterTypes = new[] { "System.String" } };
        var direct = first with { Kind = "direct", DefinitionRoot = "Game.Helper", DeclaringType = "Game.Helper", Method = "Tick", ParameterTypes = Array.Empty<string>(), ReturnType = "System.Threading.Tasks.Task" };
        var baseline = new Inventory { FormatVersion = 1, GameVersion = "test", Producers = new[] { direct, first } };
        Require(Compare(baseline, baseline with { Producers = new[] { direct, replacement } }).SequenceEqual(new[] { "removed: " + first.Key, "added: " + replacement.Key }), "Count-preserving overload substitution must report both exact signatures");
        Require(Compare(baseline, baseline with { Producers = new[] { direct } }).SequenceEqual(new[] { "removed: " + first.Key }), "Removed producers must be reported");
        Require(Compare(baseline with { Producers = new[] { direct } }, baseline).SequenceEqual(new[] { "added: " + first.Key }), "Added producers must be reported");
        Require(Compare(baseline, baseline with { Producers = new[] { first } }).SequenceEqual(new[] { "removed: " + direct.Key }), "Direct helpers require independent inventory entries");
        foreach (var altered in new[]
        {
            first with { ReturnType = "System.Int32" }, first with { DefinitionRoot = "Game.OtherRoot" },
            first with { DeclaringType = "Game.OtherModel" }, first with { Kind = "direct" }, first with { CallingConvention = "static" },
        })
            Require(Compare(baseline with { Producers = new[] { first } }, baseline with { Producers = new[] { altered } }).Length == 2, "Return, root, declaring type, producer kind, and calling convention belong to identity");
        Require(Compare(baseline, baseline with { GameVersion = "old" }).Length != 0, "Reviewed game version must match the verified pin");
        Require(Compare(baseline with { Producers = new[] { first with { ParameterTypes = new[] { "A,B" } } } },
            baseline with { Producers = new[] { first with { ParameterTypes = new[] { "A", "B" } } } }).Length == 2, "Parameter array boundaries belong to identity");
        Require(JsonSerializer.Serialize(Snapshot(gameVersion, targets.Reverse(), helpers.Reverse()), Options) == JsonSerializer.Serialize(candidate, Options), "Enumeration order must not change the candidate");
        Require(TypeName(typeof(Dictionary<string, List<int[,]>>)) == "System.Collections.Generic.Dictionary`2<System.String,System.Collections.Generic.List`1<System.Int32[,]>>"
            && TypeName(typeof(int[])) == "System.Int32[]" && TypeName(typeof(int).MakeArrayType(1)) == "System.Int32[*]", "Full generic and array types must be unambiguous");
        var tryGet = Producer.FromMethod("direct", typeof(Dictionary<string, int>).GetMethod("TryGetValue"));
        Require(tryGet.DefinitionRoot == "System.Collections.Generic.Dictionary`2<System.String,System.Int32>" && tryGet.DeclaringType == tryGet.DefinitionRoot
            && tryGet.CallingConvention == "instance" && tryGet.Method == "TryGetValue" && tryGet.ParameterTypes.SequenceEqual(new[] { "System.String", "out System.Int32&" })
            && tryGet.ReturnType == "System.Boolean", "Out parameters and constructed declaring types must retain exact identity");
        var empty = Producer.FromMethod("direct", typeof(Enumerable).GetMethod("Empty"));
        Require(empty.CallingConvention == "static" && empty.Method == "Empty<!!0>" && empty.ParameterTypes.Length == 0
            && empty.ReturnType == "System.Collections.Generic.IEnumerable`1<!!0>", "Method generic parameters must retain their positions");
        var byRef = Producer.FromMethod("direct", typeof(ProducerInventoryFixtures).GetMethod(nameof(RefParameter), BindingFlags.NonPublic | BindingFlags.Static));
        var byIn = Producer.FromMethod("direct", typeof(ProducerInventoryFixtures).GetMethod(nameof(InParameter), BindingFlags.NonPublic | BindingFlags.Static)) with { Method = byRef.Method };
        Require(byRef.ParameterTypes.SequenceEqual(new[] { "ref System.Int32&" }) && byIn.ParameterTypes.SequenceEqual(new[] { "in System.Int32&" })
            && Compare(baseline with { Producers = new[] { byRef } }, baseline with { Producers = new[] { byIn } }).Length == 2,
            "Changing ref to in must change identity despite identical reflected parameter types");

        string json = JsonSerializer.Serialize(baseline, Options);
        var reordered = JsonNode.Parse(json).AsObject();
        string equivalent = "{\"producers\":" + reordered["producers"].ToJsonString() + ",\"game_version\":\"test\",\"format_version\":1}";
        Require(Compare(Parse(equivalent), baseline).Length == 0, "JSON property order and whitespace must not change producer identity");
        foreach (var malformed in new[]
        {
            "", "null", "[]", "{}", "{invalid", json + "{}",
            json.Replace("\"format_version\": 1", "\"format_version\": \"1\"", StringComparison.Ordinal),
            json.Replace("\"format_version\": 1", "\"format_version\": 1, \"format_version\": 1", StringComparison.Ordinal),
            json.Replace("\"kind\": \"direct\"", "\"kind\": \"direct\", \"kind\": \"direct\"", StringComparison.Ordinal),
            JsonSerializer.Serialize(baseline with { FormatVersion = 2 }, Options),
            JsonSerializer.Serialize(baseline with { GameVersion = " " }, Options),
            JsonSerializer.Serialize(baseline with { Producers = new[] { first, direct } }, Options),
            JsonSerializer.Serialize(baseline with { Producers = new[] { direct, direct } }, Options),
            JsonSerializer.Serialize(baseline with { Producers = new Producer[] { null } }, Options),
        }) Reject(malformed);
        foreach (var field in new[] { "format_version", "game_version", "producers" })
        {
            var missing = JsonNode.Parse(json).AsObject();
            missing.Remove(field);
            Reject(missing.ToJsonString());
            var nulled = JsonNode.Parse(json).AsObject();
            nulled[field] = null;
            Reject(nulled.ToJsonString());
        }
        foreach (var field in new[] { "kind", "definition_root", "declaring_type", "calling_convention", "method", "parameter_types", "return_type" })
        {
            var missing = JsonNode.Parse(json);
            missing["producers"][0].AsObject().Remove(field);
            Reject(missing.ToJsonString());
            var nulled = JsonNode.Parse(json);
            nulled["producers"][0][field] = null;
            Reject(nulled.ToJsonString());
        }
        foreach (var malformed in new[]
        {
            first with { Kind = "other" }, first with { CallingConvention = "other" }, first with { Method = " " },
            first with { DeclaringType = "Game.Model\tInjected" }, first with { ParameterTypes = new[] { "" } },
            first with { ParameterTypes = new string[] { null } }, first with { ParameterTypes = new[] { "System.Int32\n" } },
        }) Reject(JsonSerializer.Serialize(baseline with { Producers = new[] { malformed } }, Options));
        var unknown = JsonNode.Parse(json);
        unknown["unexpected"] = true;
        Reject(unknown.ToJsonString());
        unknown = JsonNode.Parse(json);
        unknown["producers"][0]["unexpected"] = true;
        Reject(unknown.ToJsonString());
        unknown = JsonNode.Parse(json);
        unknown["producers"][0]["parameter_types"] = "System.Int32";
        Reject(unknown.ToJsonString());
        Console.WriteLine($"REVIEWED PRODUCER INVENTORY PASS: game={gameVersion} discovered={targets.Count} direct={helpers.Length}");
    }

    internal static void VerifyInstalled(Harmony harmony, IReadOnlyList<MethodInfo> discovered)
    {
        var targetKeys = discovered.Select(method => (method.Module, method.MetadataToken)).ToHashSet();
        var prefix = AccessTools.Method(typeof(FlowCapture), nameof(FlowCapture.Prefix));
        var installed = harmony.GetPatchedMethods().OfType<MethodInfo>()
            .Where(method => !targetKeys.Contains((method.Module, method.MetadataToken))
                && Harmony.GetPatchInfo(method).Prefixes.Any(patch => patch.owner == harmony.Id && FlowCapture.SameMethod(patch.PatchMethod, prefix)));
        var reviewed = ReadReviewed();
        reviewed = reviewed with { Producers = reviewed.Producers.Where(producer => producer.Kind == "direct").ToArray() };
        var actual = Snapshot(reviewed.GameVersion, Array.Empty<MethodInfo>(), installed);
        var changes = Compare(reviewed, actual);
        Require(changes.Length == 0, "Installed direct producers differ from the reviewed helper contracts:\n" + string.Join('\n', changes));
    }

    private static Inventory ReadReviewed()
    {
        using var stream = typeof(ProducerInventoryFixtures).Assembly.GetManifestResourceStream("SpireProfiler.ProducerInventory")
            ?? throw new InvalidDataException("Missing reviewed producer inventory resource");
        using var reader = new StreamReader(stream);
        return Parse(reader.ReadToEnd());
    }

    private static Inventory Parse(string json)
    {
        using var document = JsonDocument.Parse(json);
        var inventory = document.Deserialize<Inventory>(Options) ?? throw new InvalidDataException("Null reviewed producer inventory");
        if (inventory.FormatVersion != 1 || string.IsNullOrWhiteSpace(inventory.GameVersion) || inventory.GameVersion.Any(char.IsControl) || inventory.Producers == null)
            throw new InvalidDataException("Invalid producer inventory format version, game version, or producers");
        foreach (var element in document.RootElement.GetProperty("producers").EnumerateArray().Prepend(document.RootElement))
        {
            if (element.ValueKind != JsonValueKind.Object) throw new InvalidDataException("Producer inventory entries must be objects");
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in element.EnumerateObject())
                if (!names.Add(property.Name)) throw new InvalidDataException("Duplicate producer inventory property: " + property.Name);
        }
        string previous = null;
        foreach (var producer in inventory.Producers)
        {
            if (producer == null || producer.Kind is not ("discovered" or "direct") || producer.CallingConvention is not ("instance" or "static") || producer.ParameterTypes == null
                || new[] { producer.DefinitionRoot, producer.DeclaringType, producer.Method, producer.ReturnType }.Concat(producer.ParameterTypes)
                    .Any(value => string.IsNullOrWhiteSpace(value) || value.Any(char.IsControl)))
                throw new InvalidDataException("Malformed producer inventory entry");
            string key = producer.Key;
            if (previous != null && StringComparer.Ordinal.Compare(previous, key) >= 0)
                throw new InvalidDataException("Reviewed producers must be unique and ordinal-sorted: " + key);
            previous = key;
        }
        return inventory;
    }

    private static void Reject(string json)
    {
        try { Parse(json); }
        catch (Exception error) when (error is JsonException or InvalidDataException) { return; }
        throw new InvalidOperationException("Malformed reviewed JSON inventory was accepted: " + json);
    }

    private static int RefParameter(ref int value) => value;
    private static int InParameter(in int value) => value;

    private static Inventory Snapshot(string gameVersion, IEnumerable<MethodInfo> targets, IEnumerable<MethodInfo> helpers)
        => new()
        {
            FormatVersion = 1,
            GameVersion = gameVersion,
            Producers = targets.Select(method => Producer.FromMethod("discovered", method)).Concat(helpers.Select(method => Producer.FromMethod("direct", method)))
                .OrderBy(producer => producer.Key, StringComparer.Ordinal).ToArray(),
        };

    private static string TypeName(Type type)
    {
        if (type.IsByRef) return TypeName(type.GetElementType()) + "&";
        if (type.IsPointer) return TypeName(type.GetElementType()) + "*";
        if (type.IsArray) return TypeName(type.GetElementType()) + (type.IsSZArray ? "[]" : type.GetArrayRank() == 1 ? "[*]" : "[" + new string(',', type.GetArrayRank() - 1) + "]");
        if (type.IsGenericParameter) return (type.DeclaringMethod == null ? "!" : "!!") + type.GenericParameterPosition;
        if (type.IsGenericType) return type.GetGenericTypeDefinition().FullName + "<" + string.Join(',', type.GetGenericArguments().Select(TypeName)) + ">";
        return type.FullName ?? throw new InvalidDataException("Producer signature type has no full name: " + type);
    }

    private static string[] Compare(Inventory reviewed, Inventory candidate)
    {
        if (reviewed.GameVersion != candidate.GameVersion) return new[] { "Reviewed game version " + reviewed.GameVersion + " differs from verified " + candidate.GameVersion };
        var expected = reviewed.Producers.Select(producer => producer.Key).ToArray();
        var actual = candidate.Producers.Select(producer => producer.Key).ToArray();
        return expected.Except(actual, StringComparer.Ordinal).Select(key => "removed: " + key)
            .Concat(actual.Except(expected, StringComparer.Ordinal).Select(key => "added: " + key)).ToArray();
    }

    private static void Require(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
    }
}
