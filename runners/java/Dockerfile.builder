ARG TARGETARCH
FROM --platform=linux/${TARGETARCH} eclipse-temurin:17.0.16_8-jdk-jammy@sha256:47d330891da01c4b17fb6af5018d9a7bad33f7a252a91dde890c10d4de5ebd33 AS compile
COPY runners/java/fetch-jars.sh /usr/local/bin/sagat-fetch-jars
RUN sagat-fetch-jars
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
