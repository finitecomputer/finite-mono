FROM docker.io/searxng/searxng:latest@sha256:6c1e8797ba29a47d575dea4805e3138976bfe1333fab630418de03930bd14803

COPY settings.yml /etc/searxng/settings.yml
COPY auth_proxy.py /usr/local/bin/finite_search_auth_proxy.py
COPY entrypoint-auth-proxy.sh /usr/local/bin/entrypoint-auth-proxy.sh

RUN chmod 0755 /usr/local/bin/finite_search_auth_proxy.py /usr/local/bin/entrypoint-auth-proxy.sh

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/entrypoint-auth-proxy.sh"]
