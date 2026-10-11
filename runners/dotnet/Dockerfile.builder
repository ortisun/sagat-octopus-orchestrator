FROM mcr.microsoft.com/dotnet/sdk:10.0.401@sha256:e70cdb7f80b0348f5cb85f19a8f670fca061f033d57eed12fa003d58b0e06317 AS publish
WORKDIR /src/runners/dotnet
COPY runners/dotnet/ ./
RUN dotnet restore Sagat.DotnetBuild/Sagat.DotnetBuild.csproj \
    && dotnet publish Sagat.DotnetBuild/Sagat.DotnetBuild.csproj --no-restore -c Release -o /out/build \
    && dotnet restore Catalog/Catalog.csproj --locked-mode --packages /packages \
    && test "$(sha512sum /packages/newtonsoft.json/13.0.4/newtonsoft.json.13.0.4.nupkg | cut -d ' ' -f1)" = "6d1faff84ff227a83b195dae5f0d8ead44a36187e32e438b0bc243e24092db79ac2daa672ad7493c1240ef97f01c7fbe12b21f7de22acd82132f102eaf82805c" \
    && test "$(sha256sum /packages/newtonsoft.json/13.0.4/lib/net6.0/Newtonsoft.Json.dll | cut -d ' ' -f1)" = "a28c251dfe36d881e9e2462e171441b8b0ec156fe3f452602c9149b1b9efe05b" \
    && mkdir -p /out/catalog \
    && cp Catalog/catalog.json /out/catalog/catalog.json \
    && cp /packages/newtonsoft.json/13.0.4/lib/net6.0/Newtonsoft.Json.dll /out/catalog/Newtonsoft.Json.dll

FROM mcr.microsoft.com/dotnet/runtime:10.0.12@sha256:b89586dc17781f25531909993658aa8161205ae38b8cec8847df4a8221a403d5
WORKDIR /opt/sagat
COPY --from=publish /out/build/ ./build/
COPY --from=publish /out/catalog/ ./catalog/
ENV SAGAT_DOTNET_CATALOG=/opt/sagat/catalog/catalog.json \
    DOTNET_EnableDiagnostics=0 \
    DOTNET_NOLOGO=1
USER 65534:65534
ENTRYPOINT ["dotnet", "/opt/sagat/build/Sagat.DotnetBuild.dll"]
