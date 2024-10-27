# Rust Blogging Projects

A little web blogging platform written in Rust using `axum`, with SQLite as database.

## Running

Run these 2 commands:
```shell
docker build -t rust_blogging_project .
docker run -p 3000:3000 -d rust_blogging_project
```

And you can access the application at http://localhost:3000/home.
