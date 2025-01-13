# Session
a tool for storing ssh session. 

## Scenario
Many users of Window Terminal have trouble while logining remote host by ssh,
once you close tabs or exit Window Terminal, you will lost all ssh sessions.

Most of you may have noticed that MacOS has builtin session storage, 
so this tool does the same thing expect doesn't save password yet, 
but we are going make it available soon.

## Feature

1. Store SSH sessions
2. Load known hosts from `./sss/known_host`

> To load known hosts requires diabling ssh hashed host, hashed host is default on newer Ubuntu release.

## Get Started

Install from GitHub

```shell
cargo install --git https://github.com/zaoying/session.git
```

## Usage

First, just type `session` in your favorite terminal, then it list stored sessions:

```markdown
List stored sessions from '~/.session':
-----------------------------------------
* Enter number listed above to open session, such as '1';
* Enter negative number listed above to delete session, such as '-1';
* Enter 'username@host' to open new session;
* Enter nothing to list hosts from '~/.ssh/known_host';
```

It will be nothing in first time, because you don't have any stored sessions yet.
You should press `Enter` again, then it list known host from `~/.ssh/known_hosts` .

```yaml
1: github.com
```

Then type number at the begining of the host you want to connect with, it will remind you to input `username` for ssh.
As you press `Enter` again, it will open ssh session for you, then stores ssh session into `~/.session`.

Anyway, you could just type any vaild session like `jhondoe@domain` , 
then press `Enter`, it will also open session then save it.

> Saved sessions are not include any credentials yet for security reason, it will be available soon.
> if you use secret key to login, please make sure the path of secret key is absoutely, or it might failed.

## Road Map

* Saved credentials of ssh session.