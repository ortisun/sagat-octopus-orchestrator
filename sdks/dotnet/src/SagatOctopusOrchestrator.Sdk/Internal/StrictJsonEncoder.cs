using System.Text.Encodings.Web;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>
/// The encoder every JSON text of the SDK is written with: the relaxed one (only what JSON requires is escaped, as
/// <c>JSON.stringify</c> does), except that text that is not valid Unicode (a lone surrogate) is refused instead of being
/// replaced with U+FFFD behind the caller's back (SPEC section 8).
/// </summary>
internal sealed unsafe class StrictJsonEncoder : JavaScriptEncoder
{
    internal static readonly StrictJsonEncoder Instance = new();

    private readonly JavaScriptEncoder inner = UnsafeRelaxedJsonEscaping;

    public override int MaxOutputCharactersPerInputCharacter => inner.MaxOutputCharactersPerInputCharacter;

    public override bool WillEncode(int unicodeScalar) => inner.WillEncode(unicodeScalar);

    public override int FindFirstCharacterToEncode(char* text, int textLength)
    {
        for (var i = 0; i < textLength; i++)
        {
            var c = text[i];
            if (char.IsHighSurrogate(c) && i + 1 < textLength && char.IsLowSurrogate(text[i + 1]))
            {
                i++;
            }
            else if (char.IsSurrogate(c))
            {
                throw new ArgumentException("the text is not valid Unicode (a lone surrogate)", nameof(text));
            }
        }

        return inner.FindFirstCharacterToEncode(text, textLength);
    }

    public override bool TryEncodeUnicodeScalar(int unicodeScalar, char* buffer, int bufferLength, out int numberOfCharactersWritten) =>
        inner.TryEncodeUnicodeScalar(unicodeScalar, buffer, bufferLength, out numberOfCharactersWritten);
}
