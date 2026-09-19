FROM python:3.13-slim
WORKDIR /app
COPY triage.py .
RUN python3 triage.py selftest
USER 65532
ENTRYPOINT ["python3", "/app/triage.py"]
CMD ["sync"]
