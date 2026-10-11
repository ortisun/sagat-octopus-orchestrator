using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

public class ErrorTests
{
    [Fact]
    public void AnErrorCarriesTheCodeTheTraceAndTheRemediation()
    {
        var error = new SagatException(
            SagatErrorCode.RateLimited,
            "slow down",
            httpStatus: 429,
            traceId: new string('a', 32),
            reason: "TENANT",
            retryAfter: TimeSpan.FromSeconds(2));
        Assert.Equal(SagatErrorCode.RateLimited, error.Code);
        Assert.Equal("RATE_LIMITED", error.Code.ToWireName());
        Assert.Equal(429, error.HttpStatus);
        Assert.Null(error.GrpcStatus);
        Assert.True(error.Retryable, "from the catalog");
        Assert.Equal(SagatErrorCode.RateLimited.Remediation(), error.Remediation);
        Assert.Equal(new string('a', 32), error.TraceId);
        Assert.Equal("TENANT", error.Reason);
        Assert.Equal(TimeSpan.FromSeconds(2), error.RetryAfter);
    }

    [Fact]
    public void TheServersOwnRetryableFlagWinsOverTheCatalog()
    {
        Assert.False(new SagatException(SagatErrorCode.Unavailable, "x", retryable: false).Retryable);
        Assert.True(new SagatException(SagatErrorCode.Internal, "x", retryable: true).Retryable);
        Assert.False(new SagatException(SagatErrorCode.Internal, "x").Retryable);
        Assert.Equal(string.Empty, new SagatException(SagatErrorCode.Transport, "x").TraceId);
    }

    [Theory]
    [InlineData("BAD_REQUEST", SagatErrorCode.BadRequest)]
    [InlineData("STALE_ATTEMPT", SagatErrorCode.StaleAttempt)]
    [InlineData("IDEMPOTENCY_KEY_REUSED", SagatErrorCode.IdempotencyKeyReused)]
    [InlineData("PAYLOAD_TOO_LARGE", SagatErrorCode.PayloadTooLarge)]
    [InlineData("UNSUPPORTED_PROTOCOL", SagatErrorCode.UnsupportedProtocol)]
    public void WireNamesAreTheCatalogsSpelling(string wire, SagatErrorCode code)
    {
        Assert.Equal(wire, code.ToWireName());
        Assert.True(SagatErrorCodes.TryParse(wire, out var parsed));
        Assert.Equal(code, parsed);
    }

    [Fact]
    public void AnUnknownWireNameIsNotACode()
    {
        Assert.False(SagatErrorCodes.TryParse("NO_SUCH_CODE", out _));
        Assert.False(SagatErrorCodes.TryParse(null, out _));
    }
}
