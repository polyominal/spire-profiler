using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

using var request = JsonDocument.Parse(Console.In.ReadToEnd());
var mode = request.RootElement.GetProperty("mode").GetString();
var errors = new List<string>();
var sources = request.RootElement.GetProperty("sources").EnumerateArray().Select(source =>
{
    var name = source[0].GetString()!;
    var options = new CSharpParseOptions();
    var tree = CSharpSyntaxTree.ParseText(source[1].GetString()!, options, name);
    errors.AddRange(tree.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error).Select(d => d.ToString()));
    if (mode == "abi" && tree.GetRoot().DescendantTrivia().Any(trivia => trivia.IsKind(SyntaxKind.IfDirectiveTrivia)))
        errors.Add($"{name}: conditional compilation in ABI inputs requires an explicit symbol configuration");
    return (name, root: tree.GetRoot());
}).ToArray();

object data = mode switch
{
    "abi" => Abi.Read(sources, errors),
    "catalog" => sources.SelectMany(source => source.root.DescendantNodes().OfType<ClassDeclarationSyntax>()
        .Where(type => type.Modifiers.Any(SyntaxKind.PublicKeyword) && type.Parent is not TypeDeclarationSyntax)
        .Select(type => new
        {
            source = source.name,
            name = type.Identifier.ValueText,
            ns = string.Join(".", type.Ancestors().OfType<BaseNamespaceDeclarationSyntax>().Reverse().Select(ns => ns.Name.ToString())),
            methods = type.Members.OfType<MethodDeclarationSyntax>().Select(method => new
            {
                name = method.Identifier.ValueText,
                body = (method.Body as SyntaxNode ?? method.ExpressionBody?.Expression)?.ToString(),
                is_override = method.Modifiers.Any(SyntaxKind.OverrideKeyword),
                is_virtual = method.Modifiers.Any(SyntaxKind.PublicKeyword) && method.Modifiers.Any(SyntaxKind.VirtualKeyword),
                calls = method.DescendantNodes().OfType<InvocationExpressionSyntax>().Select(call => call.Expression switch
                {
                    SimpleNameSyntax name => name.Identifier.ValueText,
                    MemberAccessExpressionSyntax member => member.Name.Identifier.ValueText,
                    _ => ""
                }).Where(name => name.Length != 0).ToArray()
            }).ToArray()
        })).ToArray(),
    _ => throw new InvalidOperationException($"unknown syntax mode: {mode}")
};
Console.WriteLine(JsonSerializer.Serialize(new { data, errors }));

static class Abi
{
    internal sealed record Binding(string delegate_name, string export_name, string source);
    internal sealed record Signature(string[] parameters, string returns);
    internal sealed record Declaration(string name, Signature signature, string source);

    internal static object Read((string name, SyntaxNode root)[] sources, List<string> errors)
    {
        var bindings = new List<Binding>();
        foreach (var (source, root) in sources)
            foreach (var call in root.DescendantNodes().OfType<InvocationExpressionSyntax>())
            {
                var generic = call.Expression as GenericNameSyntax ?? (call.Expression as MemberAccessExpressionSyntax)?.Name as GenericNameSyntax;
                if (generic?.Identifier.ValueText != "GetExport") continue;
                var arguments = call.ArgumentList.Arguments;
                if (generic.TypeArgumentList.Arguments is [IdentifierNameSyntax target]
                    && arguments.Count == 2 && arguments.All(a => a.NameColon == null && a.RefKindKeyword.IsKind(SyntaxKind.None))
                    && arguments[0].Expression is IdentifierNameSyntax { Identifier.ValueText: "lib" }
                    && arguments[1].Expression is LiteralExpressionSyntax literal && literal.IsKind(SyntaxKind.StringLiteralExpression)
                    && ProfilerName(literal.Token.ValueText))
                    bindings.Add(new(target.Identifier.ValueText, literal.Token.ValueText, source));
                else errors.Add($"{source}: unrecognized GetExport call: {call}");
            }
        var bound = bindings.Select(binding => binding.delegate_name).ToHashSet();
        var declarations = new List<Declaration>();
        foreach (var (source, root) in sources)
            foreach (var declaration in root.DescendantNodes().OfType<DelegateDeclarationSyntax>().Where(d => bound.Contains(d.Identifier.ValueText)))
            {
                var name = declaration.Identifier.ValueText;
                try
                {
                    if (declaration.Modifiers.Any(modifier => !modifier.IsKind(SyntaxKind.PrivateKeyword))
                        || declaration.TypeParameterList != null || declaration.ConstraintClauses.Count != 0
                        || declaration.AttributeLists.Count > 1
                        || (declaration.AttributeLists.Count == 1 && !Attribute(declaration.AttributeLists, "UnmanagedFunctionPointer", "CallingConvention", "Cdecl")))
                        throw new InvalidOperationException("unsupported delegate attributes or modifiers");
                    var returns = declaration.ReturnType.ToString();
                    if (!Scalar(returns) && returns != "void")
                        throw new InvalidOperationException($"unsupported C# return type '{returns}'");
                    var parameters = declaration.ParameterList.Parameters.Select(Parameter).ToArray();
                    declarations.Add(new(name, new(parameters, returns), source));
                }
                catch (InvalidOperationException error) { errors.Add($"{source}: {name}: {error.Message}"); }
            }
        return new { bindings, delegates = declarations };
    }

    static string Parameter(ParameterSyntax parameter)
    {
        var type = parameter.Type?.ToString() ?? "";
        if (parameter.Modifiers.Count != 0 || parameter.Default != null)
            throw new InvalidOperationException($"unsupported C# parameter modifiers or default: '{parameter}'");
        if (type == "string")
        {
            if (!Attribute(parameter.AttributeLists, "MarshalAs", "UnmanagedType", "LPUTF8Str"))
                throw new InvalidOperationException("string parameter without [MarshalAs(UnmanagedType.LPUTF8Str)]");
        }
        else if (!Scalar(type) || parameter.AttributeLists.Count != 0)
            throw new InvalidOperationException($"unsupported C# parameter type or attributes '{type}'");
        return type;
    }

    static bool Attribute(SyntaxList<AttributeListSyntax> lists, string name, string owner, string value) =>
        lists is [{ Target: null, Attributes: [var attribute] }]
        && attribute.Name is IdentifierNameSyntax identifier && identifier.Identifier.ValueText == name
        && attribute.ArgumentList?.Arguments is [{ NameColon: null, NameEquals: null, Expression: MemberAccessExpressionSyntax member }]
        && member.Expression is IdentifierNameSyntax receiver && receiver.Identifier.ValueText == owner
        && member.Name.Identifier.ValueText == value;

    static bool Scalar(string type) => type is "int" or "uint" or "long" or "ulong" or "double" or "IntPtr";

    static bool ProfilerName(string name)
    {
        var separator = name.IndexOf("_profiler_", StringComparison.Ordinal);
        return separator > 0 && separator + "_profiler_".Length < name.Length
            && name.All(character => char.IsAsciiLetterOrDigit(character) || character == '_');
    }
}
