using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public class TraceparentTests
{
    private const string Valid = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

    [Fact]
    public void AW3CTraceContextIsRecognisedAndItsTraceIdRead()
    {
        Assert.True(Traceparent.IsValid(Valid));
        Assert.Equal("0af7651916cd43dd8448eb211c80319c", Traceparent.TraceIdOf(Valid));
        Assert.Equal(Valid, Traceparent.Check(Valid));
    }

    [Theory]
    [InlineData("not-a-trace")]
    [InlineData("")]
    [InlineData("00-00000000000000000000000000000000-b7ad6b7169203331-01")]
    [InlineData("00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01")]
    [InlineData("01-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")]
    [InlineData("00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01")]
    public void AMalformedOneIsABadRequestRaisedBeforeAnythingIsSent(string value)
    {
        Assert.False(Traceparent.IsValid(value));
        Assert.Null(Traceparent.TraceIdOf(value));
        var error = Assert.Throws<SagatException>(() => Traceparent.Check(value));
        Assert.Equal(SagatErrorCode.BadRequest, error.Code);
    }

    [Fact]
    public void ANewOneIsValidAndSampled()
    {
        var created = Traceparent.New();
        Assert.True(Traceparent.IsValid(created));
        Assert.EndsWith("-01", created, StringComparison.Ordinal);
        Assert.NotEqual(created, Traceparent.New());
    }
}
