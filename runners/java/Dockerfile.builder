ARG TARGETARCH
FROM --platform=linux/${TARGETARCH} eclipse-temurin:17.0.16_8-jdk-jammy@sha256:47d330891da01c4b17fb6af5018d9a7bad33f7a252a91dde890c10d4de5ebd33 AS compile
ADD --checksum=sha256:873a606e23507969f9bbbea939d5e19274a88775ea5a169ba7e2d795aa5156e1 https://repo1.maven.org/maven2/com/fasterxml/jackson/core/jackson-annotations/2.17.2/jackson-annotations-2.17.2.jar /opt/sagat/lib/jackson-annotations.jar
ADD --checksum=sha256:721a189241dab0525d9e858e5cb604d3ecc0ede081e2de77d6f34fa5779a5b46 https://repo1.maven.org/maven2/com/fasterxml/jackson/core/jackson-core/2.17.2/jackson-core-2.17.2.jar /opt/sagat/lib/jackson-core.jar
ADD --checksum=sha256:c04993f33c0f845342653784f14f38373d005280e6359db5f808701cfae73c0c https://repo1.maven.org/maven2/com/fasterxml/jackson/core/jackson-databind/2.17.2/jackson-databind-2.17.2.jar /opt/sagat/lib/jackson-databind.jar
COPY runners/java/api/ /src/api/
COPY runners/java/host/sagat/host/BuildHost.java /src/host/sagat/host/BuildHost.java
RUN chmod 0444 /opt/sagat/lib/*.jar \
    && mkdir -p /out/api /out/host \
    && javac --release 17 -proc:none -d /out/api $(find /src/api -name '*.java' -type f | sort) \
    && jar --create --file /out/sagat-inline-api.jar -C /out/api . \
    && javac --release 17 -proc:none -classpath '/opt/sagat/lib/*:/out/sagat-inline-api.jar' \
       -d /out/host /src/host/sagat/host/BuildHost.java

FROM --platform=linux/${TARGETARCH} eclipse-temurin:17.0.16_8-jdk-jammy@sha256:47d330891da01c4b17fb6af5018d9a7bad33f7a252a91dde890c10d4de5ebd33
COPY --from=compile /opt/sagat/lib/ /opt/sagat/lib/
COPY --from=compile /out/sagat-inline-api.jar /opt/sagat/api/sagat-inline-api.jar
COPY --from=compile /out/host/ /opt/sagat/host/
USER 65534:65534
ENTRYPOINT ["java", "-XX:+DisableAttachMechanism", "-cp", "/opt/sagat/host:/opt/sagat/lib/*", "sagat.host.BuildHost"]
